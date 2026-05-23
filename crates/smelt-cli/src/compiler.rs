use crate::config::{BackendType, Config, Materialization, Target};
use crate::discovery::ModelFile;
use anyhow::Result;
use smelt_core::SourcesConfig;
use smelt_db::type_inference::infer_select_column_types;
use smelt_db::{build_type_context, StaticRefSchemaProvider};
use smelt_dialect::{
    wrap_with_type_casts, AsStructEmitter, BackendCapabilities, PrintContext, SmeltFnExpander,
    SmeltPathCallExpander, SmeltPathRefResolver, SqlDialect,
};
use smelt_parser::ast::File;
use smelt_types::{DataType, TypedColumn};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Type for the pre-resolved function-body map.
///
/// Implementation lives in `smelt-runtime`; this re-export keeps existing
/// internal callers compiling unchanged.
pub use smelt_runtime::{build_fn_body_map, build_fn_body_map_from_model_files, FnBodyMap};

/// Build a source-bound map for `inject_source_filters` from a model's upstream timeseries configs.
///
/// For each dependency of the current model that has a `timeseries:` block, derives the
/// per-source `(before_secs, after_secs)` bound from the model's SQL (Form A / Form B patterns).
/// Sources without `timeseries:` (lookups) are absent from the returned map.
///
/// The map key is the **full smelt reference** as it appears in the model's SQL, e.g.
/// `smelt.silver.events_parsed`. This matches what `inject_source_filters` uses to locate
/// source references.
///
/// # Arguments
/// * `model_sql` — The model SQL with frontmatter already stripped.
/// * `model_deps` — Names of the model's upstream dependencies (e.g., `["events_parsed"]`).
/// * `dep_timeseries` — For each dependency that has `timeseries:`, maps dep name →
///   `(address_segments, partition_column)`. Address segments give the full smelt path
///   (e.g., `["silver", "events_parsed"]`).
pub fn build_source_bound_map(
    model_sql: &str,
    dep_timeseries: &HashMap<String, (Vec<String>, String)>,
) -> HashMap<String, crate::transformer::SourceBound> {
    use smelt_planner::analysis::source_bounds::{derive_model_bounds, BoundContext, BoundResult};

    if dep_timeseries.is_empty() {
        return HashMap::new();
    }

    // Build BoundContext: dep_name → partition_col
    let mut ctx = BoundContext::new();
    for (dep_name, (_segs, partition_col)) in dep_timeseries {
        ctx.add_source(dep_name, partition_col);
    }

    let raw_bounds = derive_model_bounds(model_sql, &ctx);

    let mut result = HashMap::new();
    for (dep_name, (segs, partition_col)) in dep_timeseries {
        let bound_result = raw_bounds.get(dep_name).cloned();

        let (before_secs, after_secs) = match bound_result {
            Some(BoundResult::Bounded { before, after, .. }) => (before.0, after.0),
            // Unbounded and NotDerivable are handled at the planning stage (Constraint 10);
            // by the time we reach SQL compilation they have already been refused or allowed.
            // For pushdown purposes, skip non-derivable bounds.
            Some(BoundResult::Unbounded) | Some(BoundResult::NotDerivable) | None => continue,
        };

        // Reconstruct the full smelt ref: "smelt.<segs.join(".")>"
        // This matches the literal text in the model SQL.
        let smelt_ref = format!("smelt.{}", segs.join("."));

        result.insert(
            smelt_ref,
            crate::transformer::SourceBound {
                partition_col: partition_col.clone(),
                before_secs,
                after_secs,
            },
        );
    }

    result
}

/// Build an ordered substitution vector from positional and named arguments,
/// optionally falling back to declared default values for unfilled slots.
///
/// The result is indexed by the position of each parameter.
/// Positional arguments fill the first N slots (left-to-right); named
/// arguments fill slots by matching parameter name.  Slots that are neither
/// filled by a positional nor a named arg use the declared default SQL if one
/// exists, otherwise retain the original parameter name — this keeps
/// unfilled-slot errors visible to the downstream SQL engine (the type-checker
/// has already emitted a diagnostic for missing required args).
/// Unknown named-argument keys are silently ignored (also already rejected by
/// the type-checker via `UnknownPassingParameter`).
pub fn bind_named_args(
    params: &[(String, Option<String>)],
    positional: &[String],
    named: &[(String, String)],
) -> Vec<String> {
    let n = params.len();
    // Initialise every slot: use the declared default if present, otherwise the
    // parameter name (so unfilled required-arg slots produce recognisable SQL errors).
    let mut slots: Vec<String> = params
        .iter()
        .map(|(name, default)| default.clone().unwrap_or_else(|| name.clone()))
        .collect();

    // Fill positional slots first (left-to-right).
    for (i, arg) in positional.iter().enumerate() {
        if i < n {
            slots[i] = arg.clone();
        }
    }

    // Fill named slots — overwrite only the slot for a matching param name.
    for (key, val) in named {
        if let Some(idx) = params.iter().position(|(name, _)| name == key) {
            slots[idx] = val.clone();
        }
        // Unknown keys are silently ignored (already rejected by type-checker).
    }

    slots
}

/// Substitute parameters in a function body, supporting both positional and
/// named arguments.
///
/// Positional args fill the first N parameter slots (left-to-right); named
/// args fill slots by parameter name regardless of call order.  The two forms
/// may be mixed: positional args are assigned first, then named args fill the
/// remaining (or overwrite — callers should not mix both for the same slot;
/// the type-checker rejects that pattern via `TooManyArguments`).
///
/// Unfilled slots retain the original parameter name so the downstream SQL
/// engine surfaces a clear error rather than producing a silent miscompile.
pub fn substitute_params_with_named(
    body: &str,
    params: &[(String, Option<String>)],
    positional: &[String],
    named: &[(String, String)],
) -> String {
    let resolved = bind_named_args(params, positional, named);
    let mut result = body.to_string();
    for ((param_name, _default), arg) in params.iter().zip(resolved.iter()) {
        result = replace_identifier(&result, param_name, arg);
    }
    result
}

/// Replace whole-word occurrences of `ident` with `replacement` in `text`,
/// skipping content inside single-quoted strings (SQL string literals).
fn replace_identifier(text: &str, ident: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(text.len() + replacement.len());
    let chars: Vec<char> = text.chars().collect();
    let ident_chars: Vec<char> = ident.chars().collect();
    let n = chars.len();
    let m = ident_chars.len();
    let mut i = 0;
    let mut in_string = false;

    while i < n {
        // Track single-quoted string literals.
        if chars[i] == '\'' {
            if in_string {
                // Check for '' escape (doubled quote within string)
                if i + 1 < n && chars[i + 1] == '\'' {
                    out.push('\'');
                    out.push('\'');
                    i += 2;
                    continue;
                }
                in_string = false;
            } else {
                in_string = true;
            }
            out.push(chars[i]);
            i += 1;
            continue;
        }

        if in_string {
            out.push(chars[i]);
            i += 1;
            continue;
        }

        // Check for a whole-word match of `ident` at position i.
        let before_ok = i == 0 || !is_ident_char(chars[i - 1]);
        let slice_matches = i + m <= n
            && chars[i..i + m]
                .iter()
                .zip(ident_chars.iter())
                .all(|(a, b)| a == b);
        let after_ok = i + m >= n || !is_ident_char(chars[i + m]);

        if before_ok && slice_matches && after_ok {
            out.push_str(replacement);
            i += m;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[derive(Debug, Clone)]
pub struct CompiledModel {
    pub name: String,
    pub sql: String,
    pub materialization: Materialization,
}

fn dialect_for_backend(backend_type: BackendType) -> (SqlDialect, BackendCapabilities) {
    match backend_type {
        BackendType::DuckDB => (SqlDialect::DuckDB, BackendCapabilities::duckdb()),
        BackendType::Spark => (SqlDialect::SparkSQL, BackendCapabilities::spark()),
    }
}

/// Resolve all smelt.ref() and smelt.source() calls in arbitrary SQL text by replacing
/// them with qualified table names.
pub fn resolve_refs_in_sql(sql: &str, schema: &str) -> String {
    let parse = smelt_parser::parse(sql);
    let schema_owned = schema.to_string();
    let path_ref_resolver: SmeltPathRefResolver<'static> = Box::new(move |segs: &[String]| {
        segs.last().map(|leaf| format!("{}.{}", schema_owned, leaf))
    });
    let ctx = PrintContext {
        dialect: &SqlDialect::DuckDB,
        capabilities: &BackendCapabilities::duckdb(),
        schema,
        ephemeral_models: std::collections::HashSet::new(),
        cross_engine_refs: std::collections::HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: Some(path_ref_resolver),
        smelt_path_call: None,
    };
    smelt_dialect::print(&parse.syntax(), &ctx)
}

pub struct SqlCompiler {
    config: Config,
    dialect: SqlDialect,
    capabilities: BackendCapabilities,
    /// Cross-engine refs: model_name -> parquet read expression.
    /// Set externally before compilation when cross-engine references exist.
    cross_engine_refs: HashMap<String, String>,
    /// Upstream model and seed schemas, used by `apply_type_casts` to build a
    /// populated `TypeContext` so aggregate widening rules apply correctly to
    /// `smelt.ref()` and `smelt.source()` columns.
    ///
    /// Without this, `apply_type_casts` would build an empty `TypeContext`,
    /// causing column types from refs/sources to resolve as `Unknown` and
    /// SUM/COUNT/etc. to silently narrow to BIGINT. See bug #3 in
    /// `docs/research/20260417-0.3-regression-triage.md`.
    upstream_schemas: Arc<UpstreamSchemas>,
    /// Pre-resolved `smelt.fn.*` function bodies for SQL emission.
    ///
    /// Maps the leaf function name (e.g. `"safe_div"`) to
    /// `(param_names, body_sql)`. Populated by callers that have access to
    /// the Salsa database. When `None` (the default), `smelt.fn.*` calls
    /// pass through the printer verbatim.
    fn_bodies: Option<Arc<FnBodyMap>>,
}

/// Pre-computed upstream model and seed column schemas, plus the project's
/// sources config. Built once per project (e.g. from a populated Salsa
/// `Database`) and shared across all `SqlCompiler` instances in a registry.
#[derive(Default, Clone)]
pub struct UpstreamSchemas {
    pub models: HashMap<String, Vec<(String, TypedColumn)>>,
    pub seeds: HashMap<String, Vec<(String, TypedColumn)>>,
    pub sources: SourcesConfig,
    /// Per-entity source infos discovered from standalone `.yml` files.
    /// Used by the path-ref resolver to apply `name:` overrides at SQL
    /// generation time. When non-empty, takes precedence over `sources`.
    pub per_entity_sources: Vec<smelt_core::SourceInfo>,
}

impl UpstreamSchemas {
    /// Build an `UpstreamSchemas` from a populated Salsa `Database` and the
    /// list of model files registered in it. The CLI passes this into every
    /// `SqlCompiler` so `apply_type_casts` can resolve `smelt.ref()` columns
    /// without going through Salsa itself (the batch compiler is pure).
    ///
    /// `models` is the same list that was passed to `init_db` — we use it to
    /// know which paths to query, and to recover each model's user-facing name.
    ///
    /// # Errors
    /// Returns an error if the project root contains a legacy aggregate
    /// `sources.yml` / `sources.yaml` file.  Projects must migrate to
    /// per-entity source YAMLs before building.
    pub fn from_database(
        db: &smelt_db::Database,
        project_dir: &std::path::Path,
        models: &[crate::discovery::ModelFile],
    ) -> anyhow::Result<Self> {
        // Phase 6: hard error if a legacy aggregate sources.yml exists.
        smelt_core::check_aggregate_sources_yml(project_dir)
            .map_err(|e| anyhow::anyhow!("{}", e))?;

        let workspace = smelt_db::Workspace::try_get(db).expect("workspace not initialized");

        let mut model_schemas: HashMap<String, Vec<(String, TypedColumn)>> = HashMap::new();
        for model in models {
            let Some(file) = db.source_file(&model.path) else {
                continue;
            };
            let resolved = smelt_db::resolved_model_schema(db, workspace, file);
            let cols: Vec<(String, TypedColumn)> = resolved
                .columns
                .iter()
                .map(|c| {
                    let typed = c.data_type.clone().unwrap_or(TypedColumn {
                        data_type: DataType::Unknown,
                        nullable: true,
                    });
                    (c.name.clone(), typed)
                })
                .collect();
            model_schemas.insert(model.name.clone(), cols);
        }

        // Seeds are CSV files outside the Salsa graph under the 0.26 API; load
        // them directly via the pure smelt-core helper using the project's
        // configured `paths` (defaults to ["models"] if no smelt.yml).
        // Phase 5: use with_sidecars so pinned types and ephemeral metadata are available.
        let paths = smelt_core::Config::load(project_dir)
            .map(|c| c.paths)
            .unwrap_or_else(|_| vec!["models".to_string()]);
        let mut seed_schemas: HashMap<String, Vec<(String, TypedColumn)>> = HashMap::new();
        for seed in smelt_core::discover_seed_infos_with_sidecars(project_dir, &paths) {
            let cols: Vec<(String, TypedColumn)> = seed
                .columns
                .iter()
                .map(|(name, dt)| {
                    (
                        name.clone(),
                        TypedColumn {
                            data_type: dt.clone(),
                            nullable: true,
                        },
                    )
                })
                .collect();
            seed_schemas.insert(seed.address_segments.join("_"), cols);
        }

        let sources = SourcesConfig::load(project_dir).unwrap_or_default();

        // Phase 6: discover per-entity source YAMLs for name-override resolution
        // at SQL generation time. These take precedence over the legacy
        // `SourcesConfig` when resolving `smelt.sources.*` path refs.
        let per_entity_sources = smelt_core::discover_source_infos(project_dir, &paths);

        Ok(Self {
            models: model_schemas,
            seeds: seed_schemas,
            sources,
            per_entity_sources,
        })
    }
}

impl SqlCompiler {
    pub fn new(config: Config, target: &Target) -> Self {
        let (dialect, capabilities) = dialect_for_backend(target.backend_type());
        Self {
            config,
            dialect,
            capabilities,
            cross_engine_refs: HashMap::new(),
            upstream_schemas: Arc::new(UpstreamSchemas::default()),
            fn_bodies: None,
        }
    }

    /// Set cross-engine ref mappings (model_name -> parquet read expression).
    pub fn set_cross_engine_refs(&mut self, refs: HashMap<String, String>) {
        self.cross_engine_refs = refs;
    }

    /// Provide upstream model/seed/source schemas so `apply_type_casts` can
    /// resolve `smelt.ref()` and `smelt.source()` column types correctly.
    pub fn set_upstream_schemas(&mut self, schemas: Arc<UpstreamSchemas>) {
        self.upstream_schemas = schemas;
    }

    /// Provide pre-resolved `smelt.fn.*` function bodies for SQL emission.
    ///
    /// Maps leaf function name → (param_names, body_sql). When set, `smelt.fn.*`
    /// calls in compiled models are expanded inline. When not set (the default),
    /// they pass through verbatim.
    pub fn set_function_bodies(&mut self, bodies: FnBodyMap) {
        self.fn_bodies = Some(Arc::new(bodies));
    }

    /// Like [`SqlCompiler::set_function_bodies`] but takes an already-shared
    /// `Arc<FnBodyMap>` so callers can register the same map on multiple
    /// compilers without cloning the underlying allocation.
    fn set_function_bodies_arc(&mut self, bodies: Arc<FnBodyMap>) {
        self.fn_bodies = Some(bodies);
    }

    /// Build the `smelt.as_struct` emitter, `smelt.fn.*` expander, and
    /// `smelt.<path>(args)` path-call expander closures for use in
    /// [`PrintContext`]. Pulled out of the per-`compile_*` methods so every
    /// code path (including the production `compile_with_ephemerals` path
    /// used by `commands/run.rs`) wires them identically.
    ///
    /// Returns `(None, None, None)` when there is no syntax to analyse or no
    /// function bodies / upstream schemas have been configured — preserving
    /// the previous behaviour for tests that don't set them.
    fn build_emitters(
        &self,
        syntax: &smelt_parser::syntax_kind::SyntaxNode,
    ) -> (
        Option<AsStructEmitter<'static>>,
        Option<SmeltFnExpander<'static>>,
        Option<SmeltPathCallExpander<'static>>,
    ) {
        // Build a TypeContext from the parsed file so smelt.as_struct() can
        // look up column types for each qualifier/alias in scope.
        let type_ctx = if let Some(file) = File::cast(syntax.clone()) {
            let provider = StaticRefSchemaProvider {
                models: &self.upstream_schemas.models,
                seeds: &self.upstream_schemas.seeds,
            };
            Some(build_type_context(
                &file,
                &self.upstream_schemas.sources,
                &provider,
            ))
        } else {
            None
        };

        let dialect_name = match self.dialect {
            SqlDialect::DuckDB => "duckdb",
            SqlDialect::SparkSQL => "spark",
            SqlDialect::PostgreSQL => "postgres",
        };
        let as_struct_emitter: Option<AsStructEmitter<'static>> = type_ctx.map(|tc| {
            let backend = dialect_name.to_string();
            let emitter: AsStructEmitter<'static> =
                Box::new(move |alias: &str, except: &[String]| {
                    let cols = tc.columns_for_qualifier(alias);
                    if cols.is_empty() {
                        return None;
                    }
                    let fields: Vec<(String, DataType)> = cols
                        .into_iter()
                        .filter(|(name, _)| !except.contains(&name.to_string()))
                        .map(|(name, tc_col)| (name.to_string(), tc_col.data_type.clone()))
                        .collect();
                    if fields.is_empty() {
                        return None;
                    }
                    smelt_planner::lowering::as_struct_to_sql(alias, &fields, &backend).ok()
                });
            emitter
        });

        let fn_expander: Option<SmeltFnExpander<'static>> = self.fn_bodies.as_ref().map(|bodies| {
            let bodies = Arc::clone(bodies);
            let expander: SmeltFnExpander<'static> = Box::new(
                move |fn_name: &str, positional: Vec<String>, named: Vec<(String, String)>| {
                    let (params, body_sql) = bodies.get(fn_name)?;
                    Some(substitute_params_with_named(
                        body_sql,
                        params,
                        &positional,
                        &named,
                    ))
                },
            );
            expander
        });

        // Build a path-call expander that mirrors the fn expander: the leaf
        // segment of the path is used as the function name lookup key in
        // `fn_bodies`.  When `fn_bodies` is `None` (no functions configured)
        // we still wire `Some(expander)` so that the closure is present — it
        // will return `None` for every call, causing the printer to fall back
        // to verbatim output.  This ensures production PrintContexts always
        // have `smelt_path_call: Some(...)` rather than `None`.
        let path_call_expander: Option<SmeltPathCallExpander<'static>> =
            Some(match self.fn_bodies.as_ref() {
                Some(bodies) => {
                    let bodies = Arc::clone(bodies);
                    let expander: SmeltPathCallExpander<'static> = Box::new(
                        move |segs: &[String],
                              positional: Vec<String>,
                              named: Vec<(String, String)>| {
                            let fn_name = segs.last()?;
                            let (params, body_sql) = bodies.get(fn_name)?;
                            Some(substitute_params_with_named(
                                body_sql,
                                params,
                                &positional,
                                &named,
                            ))
                        },
                    );
                    expander
                }
                None => {
                    let expander: SmeltPathCallExpander<'static> = Box::new(
                        |_segs: &[String],
                         _positional: Vec<String>,
                         _named: Vec<(String, String)>| None,
                    );
                    expander
                }
            });

        (as_struct_emitter, fn_expander, path_call_expander)
    }

    /// Build a `SmeltPathRefResolver` for a specific `schema` string.
    ///
    /// Per architecture.md §"Default materialization name mapping" (Phase 2):
    /// - All persisted paths → `{schema}.{segs.join("_")}`
    /// - `["sources", src_name, table_name]` → `src_name.table_name`
    ///   (sources.yml still active until Phase 6)
    ///
    /// Paths not matching any known namespace return `None`, leaving the
    /// node verbatim — forward-compatible with new namespaces.
    fn make_path_ref_resolver(&self, schema: &str) -> SmeltPathRefResolver<'static> {
        self.make_path_ref_resolver_with_ephemerals(schema, &HashSet::new())
    }

    /// Like `make_path_ref_resolver` but emits `__smelt_{segs.join("_")}` for
    /// any address whose joined name appears in `ephemeral_names`. Used by
    /// `compile_with_ephemerals` so that CTE-inlined ephemeral refs resolve to
    /// their CTE alias rather than a physical table name.
    fn make_path_ref_resolver_with_ephemerals(
        &self,
        schema: &str,
        ephemeral_names: &HashSet<String>,
    ) -> SmeltPathRefResolver<'static> {
        let schema = schema.to_string();
        let cross_engine_refs = self.cross_engine_refs.clone();
        let sources = self.upstream_schemas.sources.clone();
        let per_entity_sources = self.upstream_schemas.per_entity_sources.clone();
        let ephemerals = ephemeral_names.clone();

        Box::new(move |segs: &[String]| {
            match segs {
                // smelt.sources.<source_name>.<table_name> and deeper paths.
                // Phase 6: per-entity sources with a `name:` override take
                // precedence. Without an override, the legacy `<source>.<table>`
                // mapping is preserved so existing projects continue to work.
                segs if !segs.is_empty() && segs[0] == "sources" => {
                    // Per-entity source with an explicit `name:` override wins.
                    if let Some(src_info) = per_entity_sources
                        .iter()
                        .find(|s| s.address_segments.as_slice() == segs)
                    {
                        if src_info.name_override.is_some() {
                            return Some(src_info.db_name(&schema));
                        }
                        // No override — fall through to legacy mapping below so
                        // `smelt.sources.raw.orders` still resolves to `raw.orders`.
                    }

                    // Legacy sources.yml identifier override, or the default
                    // `<source_name>.<table_name>` mapping. For
                    // `["sources", "raw", "orders"]` this produces `raw.orders`.
                    if segs.len() >= 3 {
                        let source_name = &segs[segs.len() - 2];
                        let table_name = &segs[segs.len() - 1];
                        let emit_name = sources
                            .sources
                            .iter()
                            .find(|s| s.name == *source_name)
                            .and_then(|src| src.tables.iter().find(|t| t.name == *table_name))
                            .and_then(|tbl| tbl.identifier.as_deref())
                            .unwrap_or(table_name.as_str())
                            .to_string();
                        return Some(format!("{}.{}", source_name, emit_name));
                    }

                    // Unknown sources path — return default mapping.
                    Some(format!("{}.{}", schema, segs.join("_")))
                }
                // All other non-empty paths → {schema}.{segs.join("_")}
                // Ephemeral models resolve to their CTE alias.
                segs if !segs.is_empty() => {
                    let table_name = segs.join("_");
                    // Check for ephemeral CTE alias.
                    if ephemerals.contains(&table_name) {
                        return Some(format!("__smelt_{}", table_name));
                    }
                    // Check for cross-engine parquet expression.
                    if let Some(parquet_expr) = cross_engine_refs.get(&table_name) {
                        return Some(parquet_expr.clone());
                    }
                    Some(format!("{}.{}", schema, table_name))
                }
                _ => None,
            }
        })
    }

    /// Compile a model's SQL by replacing smelt.ref() calls with table references
    pub fn compile(&self, model: &ModelFile, schema: &str) -> Result<CompiledModel> {
        // Strip frontmatter to avoid parse errors from YAML metadata
        let clean_content = smelt_parser::strip_frontmatter(&model.content);
        let parse = smelt_parser::parse(&clean_content);

        let (as_struct_emitter, fn_expander, path_call_expander) =
            self.build_emitters(&parse.syntax());

        let ctx = PrintContext {
            dialect: &self.dialect,
            capabilities: &self.capabilities,
            schema,
            ephemeral_models: std::collections::HashSet::new(),
            cross_engine_refs: self.cross_engine_refs.clone(),
            smelt_as_struct: as_struct_emitter,
            smelt_fn: fn_expander,
            smelt_path_ref: Some(self.make_path_ref_resolver(schema)),
            smelt_path_call: path_call_expander,
        };
        let compiled_sql = smelt_dialect::print(&parse.syntax(), &ctx);

        // Type-conforming cast insertion: wrap SELECT columns with CASTs so
        // backend output types match smelt's type inference exactly.
        let compiled_sql = self.apply_type_casts(&compiled_sql);

        // Get materialization: SQL metadata > smelt.yml > default
        let materialization = self.config.get_materialization_with_metadata(
            &model.name,
            model.metadata.as_ref().map(|b| b.as_ref()),
        );

        Ok(CompiledModel {
            // Use the full address-based DB name (e.g. "staging_stg_events")
            // so the backend creates/accesses the correct table.
            name: model.db_name_owned(),
            sql: compiled_sql,
            materialization,
        })
    }

    /// Wrap SELECT columns with CASTs based on type inference.
    ///
    /// Returns the original SQL unchanged if type inference can't extract
    /// column names/types (e.g. models referencing other models via smelt.ref()
    /// where upstream schemas aren't yet available).
    fn apply_type_casts(&self, sql: &str) -> String {
        let parse = smelt_parser::parse(sql);
        let file = match File::cast(parse.syntax()) {
            Some(f) => f,
            None => return sql.to_string(),
        };
        let select_stmt = match file.select_stmt() {
            Some(s) => s,
            None => return sql.to_string(),
        };

        // Build a populated TypeContext from upstream model/seed/source schemas
        // so SUM/COUNT/AVG over `smelt.upstream.col` resolve correctly.
        // Without this populated context, every ref column resolves to Unknown
        // and SUM falls through to BIGINT — silently corrupting financial
        // aggregates. See bug #3 in
        // `docs/research/20260417-0.3-regression-triage.md`.
        let provider = StaticRefSchemaProvider {
            models: &self.upstream_schemas.models,
            seeds: &self.upstream_schemas.seeds,
        };
        let ctx = build_type_context(&file, &self.upstream_schemas.sources, &provider);
        let column_types = infer_select_column_types(&select_stmt, &ctx);

        let select_list = match select_stmt.select_list() {
            Some(sl) => sl,
            None => return sql.to_string(),
        };
        let items: Vec<_> = select_list.items().collect();

        // Only apply casts if we have concrete types for at least one column
        let has_concrete = column_types
            .iter()
            .any(|tc| !matches!(tc.data_type, DataType::Unknown | DataType::Null));
        if !has_concrete {
            return sql.to_string();
        }

        let col_names: Vec<String> = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                item.alias().unwrap_or_else(|| {
                    // Fallback: infer name from expression (e.g. bare column ref "user_id")
                    item.expression()
                        .and_then(|e| e.infer_name())
                        .unwrap_or_else(|| format!("_col{}", i + 1))
                })
            })
            .collect();
        let col_name_refs: Vec<&str> = col_names.iter().map(|s| s.as_str()).collect();
        let col_type_refs: Vec<DataType> =
            column_types.iter().map(|tc| tc.data_type.clone()).collect();

        wrap_with_type_casts(sql, &col_name_refs, &col_type_refs)
    }

    /// Compile a model with custom SQL (e.g., for transformed queries).
    /// This is used for incremental processing where the SQL has been transformed.
    pub fn compile_with_sql(
        &self,
        model: &ModelFile,
        schema: &str,
        sql: &str,
    ) -> Result<CompiledModel> {
        let parse = smelt_parser::parse(sql);
        let (as_struct_emitter, fn_expander, path_call_expander) =
            self.build_emitters(&parse.syntax());
        let ctx = PrintContext {
            dialect: &self.dialect,
            capabilities: &self.capabilities,
            schema,
            ephemeral_models: std::collections::HashSet::new(),
            cross_engine_refs: self.cross_engine_refs.clone(),
            smelt_as_struct: as_struct_emitter,
            smelt_fn: fn_expander,
            smelt_path_ref: Some(self.make_path_ref_resolver(schema)),
            smelt_path_call: path_call_expander,
        };
        let compiled_sql = smelt_dialect::print(&parse.syntax(), &ctx);

        // Get materialization: SQL metadata > smelt.yml > default
        let materialization = self.config.get_materialization_with_metadata(
            &model.name,
            model.metadata.as_ref().map(|b| b.as_ref()),
        );

        Ok(CompiledModel {
            // Use the full address-based DB name (e.g. "staging_stg_events").
            name: model.db_name_owned(),
            sql: compiled_sql,
            materialization,
        })
    }

    /// Build an `EphemeralResolver` using this compiler's dialect/capabilities.
    pub fn build_ephemeral_resolver(
        &self,
        ephemeral_models: &[(String, String)],
        schema: &str,
    ) -> EphemeralResolver {
        EphemeralResolver::new(ephemeral_models, &self.dialect, &self.capabilities, schema)
    }

    /// Like `compile_with_sql`, but also inlines referenced ephemeral models as CTEs.
    pub fn compile_with_sql_and_ephemerals(
        &self,
        model: &ModelFile,
        schema: &str,
        sql: &str,
        resolver: &EphemeralResolver,
    ) -> Result<CompiledModel> {
        let ephemeral_refs: HashSet<&str> = resolver
            .ephemeral_names
            .iter()
            .map(|s| s.as_str())
            .collect();

        let parse = smelt_parser::parse(sql);
        let (as_struct_emitter, fn_expander, path_call_expander) =
            self.build_emitters(&parse.syntax());
        let ctx = PrintContext {
            dialect: &self.dialect,
            capabilities: &self.capabilities,
            schema,
            ephemeral_models: ephemeral_refs,
            cross_engine_refs: self.cross_engine_refs.clone(),
            smelt_as_struct: as_struct_emitter,
            smelt_fn: fn_expander,
            // Use ephemeral-aware resolver so smelt.<ephemeral> → __smelt_<name>
            smelt_path_ref: Some(
                self.make_path_ref_resolver_with_ephemerals(schema, &resolver.ephemeral_names),
            ),
            smelt_path_call: path_call_expander,
        };
        let compiled_sql = smelt_dialect::print(&parse.syntax(), &ctx);
        let compiled_sql = self.apply_type_casts(&compiled_sql);

        // Collect which ephemeral models this model references.
        // Multi-segment refs (e.g. `smelt.lookup.regions`) have a canonical
        // ephemeral name formed by joining all path segments with `_`
        // ("lookup_regions"), not just the leaf ("regions").
        let referenced: Vec<String> = model
            .refs
            .iter()
            .filter_map(|r| {
                let path_key = r.smelt_ref.to_path().join("_");
                if resolver.ephemeral_names.contains(&path_key) {
                    Some(path_key)
                } else if resolver.ephemeral_names.contains(&r.model_name) {
                    Some(r.model_name.clone())
                } else {
                    None
                }
            })
            .collect();

        let final_sql = if referenced.is_empty() {
            compiled_sql
        } else {
            let referenced_refs: Vec<&str> = referenced.iter().map(|s| s.as_str()).collect();
            let cte_list = resolver.get_cte_list(&referenced_refs);
            prepend_ephemeral_ctes(&compiled_sql, &cte_list)
        };

        let materialization = self.config.get_materialization_with_metadata(
            &model.name,
            model.metadata.as_ref().map(|b| b.as_ref()),
        );

        Ok(CompiledModel {
            // Use the full address-based DB name (e.g. "staging_stg_events").
            name: model.db_name_owned(),
            sql: final_sql,
            materialization,
        })
    }
}

/// Resolved ephemeral models ready for CTE inlining.
///
/// Holds the compiled SQL for each ephemeral model, with refs to other
/// ephemeral models already resolved as `__smelt_{name}` CTE names.
/// Internal CTEs of ephemeral models are hoisted and namespaced.
#[derive(Debug)]
pub struct EphemeralResolver {
    /// Ephemeral model names in topological order (dependencies first).
    pub order: Vec<String>,
    /// model_name -> list of (cte_alias, cte_body) pairs.
    /// For a simple ephemeral model, this is `[("__smelt_model", "SELECT ...")]`.
    /// For one with internal CTEs, the internal CTEs come first:
    /// `[("__smelt_model__cleaned", "SELECT ..."), ("__smelt_model", "SELECT ... FROM __smelt_model__cleaned")]`.
    cte_fragments: HashMap<String, Vec<(String, String)>>,
    /// Set of ephemeral model names (for quick lookup).
    pub ephemeral_names: HashSet<String>,
}

impl EphemeralResolver {
    /// Create an empty resolver (no ephemeral models).
    pub fn empty() -> Self {
        Self {
            order: Vec::new(),
            cte_fragments: HashMap::new(),
            ephemeral_names: HashSet::new(),
        }
    }

    /// Build an EphemeralResolver from a set of ephemeral models.
    ///
    /// Models must be provided in topological order (dependencies first).
    pub fn new(
        ephemeral_models: &[(String, String)], // (name, raw_sql) in topological order
        dialect: &SqlDialect,
        capabilities: &BackendCapabilities,
        schema: &str,
    ) -> Self {
        let ephemeral_names: HashSet<String> =
            ephemeral_models.iter().map(|(n, _)| n.clone()).collect();

        let mut cte_fragments: HashMap<String, Vec<(String, String)>> = HashMap::new();
        let mut order = Vec::new();

        for (model_name, raw_sql) in ephemeral_models {
            order.push(model_name.clone());
            let fragments = Self::compile_ephemeral(
                model_name,
                raw_sql,
                &ephemeral_names,
                dialect,
                capabilities,
                schema,
            );
            cte_fragments.insert(model_name.clone(), fragments);
        }

        Self {
            order,
            cte_fragments,
            ephemeral_names,
        }
    }

    /// Compile a single ephemeral model's SQL into CTE fragments.
    ///
    /// For a model without internal CTEs: produces `[("__smelt_model", "SELECT ...")]`.
    /// For a model with internal CTEs: hoists them with namespaced names:
    /// `[("__smelt_model__cte1", "..."), ("__smelt_model", "SELECT FROM __smelt_model__cte1")]`.
    fn compile_ephemeral(
        model_name: &str,
        raw_sql: &str,
        ephemeral_names: &HashSet<String>,
        dialect: &SqlDialect,
        capabilities: &BackendCapabilities,
        schema: &str,
    ) -> Vec<(String, String)> {
        let ephemeral_refs: HashSet<&str> = ephemeral_names.iter().map(|s| s.as_str()).collect();
        let clean_sql = smelt_parser::strip_frontmatter(raw_sql);
        let parse = smelt_parser::parse(&clean_sql);

        // Build a path-ref resolver that maps smelt.<path> to either
        // __smelt_<segs.join("_")> (if ephemeral) or schema.<segs.join("_")>.
        let ephemerals_owned: HashSet<String> = ephemeral_names.clone();
        let schema_owned = schema.to_string();
        let path_ref_resolver: SmeltPathRefResolver<'static> =
            Box::new(move |segs: &[String]| match segs {
                // sources stay as <source_name>.<table_name> (Phase 6 will change this)
                [ns, source_name, table_name] if ns == "sources" => {
                    Some(format!("{}.{}", source_name, table_name))
                }
                // All other non-empty paths → either ephemeral CTE or schema.table
                segs if !segs.is_empty() => {
                    let table_name = segs.join("_");
                    if ephemerals_owned.contains(&table_name) {
                        Some(format!("__smelt_{}", table_name))
                    } else {
                        Some(format!("{}.{}", schema_owned, table_name))
                    }
                }
                _ => None,
            });

        // Compile with ephemeral refs resolved to __smelt_ names
        let ctx = PrintContext {
            dialect,
            capabilities,
            schema,
            ephemeral_models: ephemeral_refs,
            cross_engine_refs: std::collections::HashMap::new(),
            smelt_as_struct: None,
            smelt_fn: None,
            smelt_path_ref: Some(path_ref_resolver),
            smelt_path_call: None,
        };
        let compiled = smelt_dialect::print(&parse.syntax(), &ctx);

        // Check for internal CTEs by parsing the compiled output
        let file = File::cast(parse.syntax());
        let select_stmt = file.as_ref().and_then(|f| f.select_stmt());
        let has_with = select_stmt.as_ref().and_then(|s| s.with_clause()).is_some();

        if !has_with {
            // Simple case — no internal CTEs
            let alias = format!("__smelt_{}", model_name);
            return vec![(alias, compiled)];
        }

        // Has internal CTEs — extract CTE names, namespace them, and hoist
        let internal_cte_names: Vec<String> = select_stmt
            .as_ref()
            .and_then(|s| s.with_clause())
            .map(|w| w.ctes().filter_map(|c| c.name()).collect())
            .unwrap_or_default();

        // Build rename map: cte_name -> __smelt_model__cte_name
        let mut rename_map: Vec<(String, String)> = Vec::new();
        for cte_name in &internal_cte_names {
            let namespaced = format!("__smelt_{}__{}", model_name, cte_name);
            rename_map.push((cte_name.clone(), namespaced));
        }

        // Apply renames to the full compiled SQL
        let mut renamed = compiled;
        for (old_name, new_name) in &rename_map {
            renamed = rename_table_references(&renamed, old_name, new_name);
        }

        // Now parse the renamed SQL to extract individual CTEs and main body
        let parts = extract_cte_parts(&renamed);

        let mut fragments: Vec<(String, String)> = Vec::new();
        for (cte_name, cte_body) in &parts.ctes {
            fragments.push((cte_name.clone(), cte_body.clone()));
        }
        let alias = format!("__smelt_{}", model_name);
        fragments.push((alias, parts.main_body));

        fragments
    }

    /// Add pre-built CTE fragments for ephemeral seeds.
    ///
    /// Each entry in `seed_ctes` is `(cte_alias, cte_body)` where:
    /// - `cte_alias` is `__smelt_<address_segments.join("_")>` (with column names for DuckDB named CTE)
    /// - `cte_body` is the VALUES literal (without the surrounding `(…)`)
    ///
    /// These are added to the resolver's fragment map and their names to `ephemeral_names`
    /// so that the path-ref resolver will emit `__smelt_<name>` when it encounters them.
    pub fn add_seed_ctes(&mut self, seed_ctes: Vec<(String, String, String)>) {
        // seed_ctes: Vec<(canonical_name, cte_alias_with_cols, cte_body)>
        // canonical_name: address_segments.join("_") — the key for ephemeral_names and cte_fragments
        for (canonical_name, alias_with_cols, body) in seed_ctes {
            self.ephemeral_names.insert(canonical_name.clone());
            self.order.push(canonical_name.clone());
            // Store as (alias_with_cols, body) so get_cte_list can emit them correctly.
            self.cte_fragments
                .insert(canonical_name, vec![(alias_with_cols, body)]);
        }
    }

    /// Get the flattened CTE list for a model that references ephemeral models.
    ///
    /// Returns (cte_alias, cte_body) pairs in correct topological order,
    /// deduplicated (each ephemeral appears once even if referenced multiple times).
    pub fn get_cte_list(&self, referenced_ephemerals: &[&str]) -> Vec<(String, String)> {
        let mut seen = HashSet::new();
        let mut result = Vec::new();

        // Walk in topological order, only including referenced ephemerals
        // and their transitive dependencies
        let needed: HashSet<&str> = self.collect_transitive_deps(referenced_ephemerals);

        for model_name in &self.order {
            if needed.contains(model_name.as_str()) && seen.insert(model_name.clone()) {
                if let Some(fragments) = self.cte_fragments.get(model_name) {
                    result.extend(fragments.iter().cloned());
                }
            }
        }

        result
    }

    /// Collect transitive ephemeral dependencies.
    fn collect_transitive_deps<'a>(&'a self, roots: &[&'a str]) -> HashSet<&'a str> {
        let mut needed: HashSet<&str> = HashSet::new();
        let mut queue: Vec<&str> = roots.to_vec();

        while let Some(name) = queue.pop() {
            if !self.ephemeral_names.contains(name) || !needed.insert(name) {
                continue;
            }
            // Check if this ephemeral model references other ephemerals
            // by looking at its CTE fragments for __smelt_ prefixed references
            if let Some(fragments) = self.cte_fragments.get(name) {
                for (_, body) in fragments {
                    for other in &self.ephemeral_names {
                        let cte_ref = format!("__smelt_{}", other);
                        if body.contains(&cte_ref) {
                            queue.push(other.as_str());
                        }
                    }
                }
            }
        }

        needed
    }
}

/// Prepend ephemeral CTEs to a compiled SQL string.
///
/// Handles merging with existing WITH clauses in the model's SQL.
pub fn prepend_ephemeral_ctes(sql: &str, cte_list: &[(String, String)]) -> String {
    if cte_list.is_empty() {
        return sql.to_string();
    }

    let mut cte_parts: Vec<String> = Vec::new();
    for (alias, body) in cte_list {
        let trimmed = body.trim();
        // Wrap in parens if not already wrapped
        if trimmed.starts_with('(') {
            cte_parts.push(format!("{} AS {}", alias, trimmed));
        } else {
            cte_parts.push(format!("{} AS (\n{}\n)", alias, trimmed));
        }
    }

    let trimmed_sql = sql.trim_start();

    // Check if the model already has a WITH clause
    let upper = trimmed_sql.to_uppercase();
    if upper.starts_with("WITH ") {
        // Merge: strip "WITH " from the model's SQL and prepend our CTEs
        let rest = &trimmed_sql[5..]; // Skip "WITH "
                                      // Check for RECURSIVE
        let rest_upper = rest.trim_start().to_uppercase();
        if rest_upper.starts_with("RECURSIVE ") {
            // User has WITH RECURSIVE — our non-recursive CTEs go before
            let after_recursive = &rest.trim_start()[10..]; // Skip "RECURSIVE "
            format!(
                "WITH {}, RECURSIVE {}",
                cte_parts.join(", "),
                after_recursive
            )
        } else {
            format!("WITH {}, {}", cte_parts.join(", "), rest)
        }
    } else {
        format!("WITH {}\n{}", cte_parts.join(", "), trimmed_sql)
    }
}

/// Parsed CTE parts from a SQL string.
struct CteParts {
    /// Individual CTEs: (name, body_sql)
    ctes: Vec<(String, String)>,
    /// The main SELECT after all CTEs
    main_body: String,
}

/// Extract CTE definitions and main body from a SQL string that starts with WITH.
///
/// Returns individual (cte_name, cte_body) pairs and the remaining SELECT.
fn extract_cte_parts(sql: &str) -> CteParts {
    let trimmed = sql.trim_start();
    let upper = trimmed.to_uppercase();
    if !upper.starts_with("WITH ") {
        return CteParts {
            ctes: vec![],
            main_body: sql.to_string(),
        };
    }

    // Skip "WITH " (and optional "RECURSIVE ")
    let mut pos = 5; // Skip "WITH "
    let rest_upper = trimmed[pos..].trim_start().to_uppercase();
    if rest_upper.starts_with("RECURSIVE ") {
        pos += trimmed[pos..]
            .find("RECURSIVE")
            .expect("starts_with check above guarantees RECURSIVE is present")
            + 10;
    }

    let bytes = trimmed.as_bytes();
    let mut ctes = Vec::new();

    loop {
        // Skip whitespace
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }

        // Read CTE name (identifier)
        let name_start = pos;
        while pos < bytes.len() && (bytes[pos].is_ascii_alphanumeric() || bytes[pos] == b'_') {
            pos += 1;
        }
        let cte_name = trimmed[name_start..pos].to_string();

        // Skip whitespace and "AS"
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        // Skip optional column list in parens before AS
        if pos < bytes.len() && bytes[pos] == b'(' {
            // This might be a column list or the CTE body — peek ahead for AS
            let paren_start = pos;
            let mut depth = 1;
            let mut pp = pos + 1;
            while pp < bytes.len() && depth > 0 {
                match bytes[pp] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    b'\'' => {
                        pp += 1;
                        while pp < bytes.len() && bytes[pp] != b'\'' {
                            pp += 1;
                        }
                    }
                    _ => {}
                }
                pp += 1;
            }
            // Check if AS follows this paren group
            let after = trimmed[pp..].trim_start().to_uppercase();
            if after.starts_with("AS") {
                // This was a column list, skip it
                pos = pp;
            } else {
                // No AS after parens — this shouldn't happen in valid SQL
                pos = paren_start;
            }
        }

        // Skip "AS" keyword
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if trimmed[pos..].to_uppercase().starts_with("AS") {
            pos += 2;
        }
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }

        // Read CTE body (in parens)
        if pos < bytes.len() && bytes[pos] == b'(' {
            let body_start = pos + 1;
            let mut depth = 1;
            pos += 1;
            while pos < bytes.len() && depth > 0 {
                match bytes[pos] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    b'\'' => {
                        pos += 1;
                        while pos < bytes.len() && bytes[pos] != b'\'' {
                            pos += 1;
                        }
                    }
                    _ => {}
                }
                pos += 1;
            }
            let body_end = pos - 1; // Exclude closing paren
            let body = trimmed[body_start..body_end].trim().to_string();
            ctes.push((cte_name, body));
        }

        // Skip whitespace
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }

        // Check for comma (more CTEs) or end of WITH clause
        if pos < bytes.len() && bytes[pos] == b',' {
            pos += 1; // Skip comma, continue to next CTE
        } else {
            break; // No more CTEs — rest is the main body
        }
    }

    let main_body = trimmed[pos..].trim().to_string();
    CteParts { ctes, main_body }
}

/// Rename table references in SQL text.
/// Simple string replacement — replaces word-boundary occurrences of `old_name` with `new_name`.
fn rename_table_references(sql: &str, old_name: &str, new_name: &str) -> String {
    // Use word-boundary-aware replacement to avoid replacing substrings
    let mut result = String::new();
    let mut remaining = sql;

    while let Some(pos) = remaining.find(old_name) {
        // Check that it's a word boundary (not part of a larger identifier)
        let before_ok = pos == 0
            || !remaining.as_bytes()[pos - 1].is_ascii_alphanumeric()
                && remaining.as_bytes()[pos - 1] != b'_';
        let after_pos = pos + old_name.len();
        let after_ok = after_pos >= remaining.len()
            || (!remaining.as_bytes()[after_pos].is_ascii_alphanumeric()
                && remaining.as_bytes()[after_pos] != b'_');

        if before_ok && after_ok {
            result.push_str(&remaining[..pos]);
            result.push_str(new_name);
            remaining = &remaining[after_pos..];
        } else {
            result.push_str(&remaining[..after_pos]);
            remaining = &remaining[after_pos..];
        }
    }

    result.push_str(remaining);
    result
}

impl SqlCompiler {
    /// Compile a model with ephemeral CTE inlining.
    ///
    /// Like `compile()`, but also inlines any referenced ephemeral models as CTEs
    /// with `__smelt_` namespaced aliases.
    pub fn compile_with_ephemerals(
        &self,
        model: &ModelFile,
        schema: &str,
        resolver: &EphemeralResolver,
    ) -> Result<CompiledModel> {
        let clean_content = smelt_parser::strip_frontmatter(&model.content);
        let parse = smelt_parser::parse(&clean_content);

        // Build ephemeral set for the printer
        let ephemeral_refs: HashSet<&str> = resolver
            .ephemeral_names
            .iter()
            .map(|s| s.as_str())
            .collect();

        let (as_struct_emitter, fn_expander, path_call_expander) =
            self.build_emitters(&parse.syntax());

        let ctx = PrintContext {
            dialect: &self.dialect,
            capabilities: &self.capabilities,
            schema,
            ephemeral_models: ephemeral_refs,
            cross_engine_refs: self.cross_engine_refs.clone(),
            smelt_as_struct: as_struct_emitter,
            smelt_fn: fn_expander,
            // Use ephemeral-aware resolver so smelt.<ephemeral> → __smelt_<name>
            smelt_path_ref: Some(
                self.make_path_ref_resolver_with_ephemerals(schema, &resolver.ephemeral_names),
            ),
            smelt_path_call: path_call_expander,
        };
        let compiled_sql = smelt_dialect::print(&parse.syntax(), &ctx);
        let compiled_sql = self.apply_type_casts(&compiled_sql);

        // Collect which ephemeral models this model references.
        //
        // For single-segment refs (e.g. `smelt.cleaned_orders`), `model_name`
        // is the leaf and matches the ephemeral name directly.
        // For multi-segment refs (e.g. `smelt.lookup.regions`), the canonical
        // ephemeral name is the segments joined with `_` ("lookup_regions"),
        // not just the leaf ("regions"). We check the joined path first.
        let referenced: Vec<String> = model
            .refs
            .iter()
            .filter_map(|r| {
                let path_key = r.smelt_ref.to_path().join("_");
                if resolver.ephemeral_names.contains(&path_key) {
                    Some(path_key)
                } else if resolver.ephemeral_names.contains(&r.model_name) {
                    Some(r.model_name.clone())
                } else {
                    None
                }
            })
            .collect();

        // Prepend ephemeral CTEs if any
        let final_sql = if referenced.is_empty() {
            compiled_sql
        } else {
            let referenced_refs: Vec<&str> = referenced.iter().map(|s| s.as_str()).collect();
            let cte_list = resolver.get_cte_list(&referenced_refs);
            prepend_ephemeral_ctes(&compiled_sql, &cte_list)
        };

        let materialization = self.config.get_materialization_with_metadata(
            &model.name,
            model.metadata.as_ref().map(|b| b.as_ref()),
        );

        Ok(CompiledModel {
            // Use the full address-based DB name (e.g. "staging_stg_events").
            name: model.db_name_owned(),
            sql: final_sql,
            materialization,
        })
    }
}

/// Registry of SQL compilers, one per target.
///
/// Each target may have a different dialect (DuckDB vs Spark), so we need
/// one compiler per target to emit correct SQL.
pub struct CompilerRegistry {
    compilers: HashMap<String, SqlCompiler>,
}

impl CompilerRegistry {
    /// Create compilers for all targets in the set.
    pub fn new(config: &Config, targets: &HashMap<String, Target>) -> Self {
        let mut compilers = HashMap::new();
        for (name, target) in targets {
            compilers.insert(name.clone(), SqlCompiler::new(config.clone(), target));
        }
        Self { compilers }
    }

    /// Get the compiler for a target name.
    pub fn get(&self, target_name: &str) -> &SqlCompiler {
        &self.compilers[target_name]
    }

    /// Set cross-engine ref mappings for a specific target's compiler.
    pub fn set_cross_engine_refs(&mut self, target_name: &str, refs: HashMap<String, String>) {
        if let Some(compiler) = self.compilers.get_mut(target_name) {
            compiler.set_cross_engine_refs(refs);
        }
    }

    /// Set the upstream model/seed/source schemas on every compiler in the
    /// registry. Schemas are computed once per project and shared across
    /// targets, since `apply_type_casts` only needs to know what columns each
    /// `smelt.ref()` / `smelt.source()` provides — it doesn't care which
    /// backend ultimately materialises the upstream model.
    pub fn set_upstream_schemas_all(&mut self, schemas: Arc<UpstreamSchemas>) {
        for compiler in self.compilers.values_mut() {
            compiler.set_upstream_schemas(schemas.clone());
        }
    }

    /// Set pre-resolved `smelt.fn.*` function bodies on every compiler in the
    /// registry. Bodies are computed once per project (via
    /// [`build_fn_body_map`]) and shared across targets so that every backend
    /// expands `smelt.fn.*` calls consistently. The single `Arc` is cloned
    /// per compiler, avoiding a fresh allocation per target.
    pub fn set_function_bodies_all(&mut self, bodies: FnBodyMap) {
        let bodies = Arc::new(bodies);
        for compiler in self.compilers.values_mut() {
            compiler.set_function_bodies_arc(bodies.clone());
        }
    }
}

// `build_fn_body_map`, `build_fn_body_map_from_model_files`, and `FnBodyMap`
// live in `smelt-runtime` (`smelt_runtime::fn_bodies`) and are re-exported
// at the top of this file.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ModelConfig, Target};
    use crate::discovery::RefInfo;
    use std::collections::HashMap;

    fn make_test_target() -> Target {
        Target {
            target_type: "duckdb".to_string(),
            database: Some("test.duckdb".to_string()),
            schema: "main".to_string(),
            connect_url: None,
            catalog: None,
            warehouse: None,
            format: None,
        }
    }

    /// Helper function to parse SQL and extract refs with real TextRange values
    fn extract_refs_from_sql(sql: &str) -> Vec<RefInfo> {
        let parse = smelt_parser::parse(sql);
        if let Some(file) = smelt_parser::File::cast(parse.syntax()) {
            smelt_core::extract_refs(&file)
        } else {
            Vec::new()
        }
    }

    fn make_test_config() -> Config {
        let mut targets = HashMap::new();
        targets.insert(
            "dev".to_string(),
            Target {
                target_type: "duckdb".to_string(),
                database: Some("test.duckdb".to_string()),
                schema: "main".to_string(),
                connect_url: None,
                catalog: None,
                warehouse: None,
                format: None,
            },
        );

        Config {
            name: "test".to_string(),
            version: 1,
            paths: vec!["models".to_string()],
            targets,
            default_materialization: Materialization::View,
            models: HashMap::new(),
            python: None,
        }
    }

    #[test]
    fn test_simple_ref_replacement() {
        let sql = r#"
SELECT
    user_id,
    COUNT(*) as session_count
FROM smelt.raw_events
GROUP BY user_id
"#;

        let model = ModelFile {
            name: "user_stats".to_string(),
            path: "models/user_stats.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(compiled.sql.contains("FROM main.raw_events"));
        assert!(!compiled.sql.contains("smelt.raw_events"));
    }

    #[test]
    fn test_multiple_refs() {
        let sql = r#"
SELECT a.user_id, b.session_id
FROM smelt.model_a a
JOIN smelt.model_b b ON a.id = b.id
"#;

        let model = ModelFile {
            name: "combined".to_string(),
            path: "models/combined.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(compiled.sql.contains("FROM main.model_a a"));
        assert!(compiled.sql.contains("JOIN main.model_b b"));
        assert!(!compiled.sql.contains("smelt.models"));
    }

    #[test]
    fn test_named_params_no_longer_rejected_at_compiler_layer() {
        // Named parameters (`param => value`) are now valid on function calls
        // (`smelt.functions.foo(param => val)`).  The compiler no longer rejects
        // refs that carry `has_named_params: true` — the type-checker has already
        // emitted any relevant diagnostics (UnknownPassingParameter, MissingArgument)
        // before the lowering layer runs.  This test verifies the guard is gone:
        // compile() succeeds even when a RefInfo carries has_named_params=true.
        use smelt_core::refs::SmeltRef;
        let sql = "SELECT user_id FROM smelt.raw_events";

        let named_ref = RefInfo {
            model_name: "raw_events".to_string(),
            has_named_params: true,
            range: rowan::TextRange::new(0.into(), 1.into()),
            smelt_ref: SmeltRef::Path(vec!["functions".to_string(), "my_fn".to_string()]),
        };
        let model = ModelFile {
            name: "filtered".to_string(),
            path: "models/filtered.sql".into(),
            content: sql.to_string(),
            refs: vec![named_ref],
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        // Must succeed — the compiler no longer rejects has_named_params=true.
        let result = compiler.compile(&model, "main");
        assert!(
            result.is_ok(),
            "compile() must not reject has_named_params=true refs (function calls with named args are valid): {:?}",
            result.err()
        );
    }

    #[test]
    fn test_materialization_from_config() {
        let model = ModelFile {
            name: "test_model".to_string(),
            path: "models/test_model.sql".into(),
            content: "SELECT 1".to_string(),
            refs: vec![],
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let mut config = make_test_config();
        config.models.insert(
            "test_model".to_string(),
            ModelConfig {
                materialization: Some(Materialization::Table),
                timeseries: None,
                incremental: None,
                tags: Vec::new(),
                target: None,
            },
        );

        let compiler = SqlCompiler::new(config, &make_test_target());
        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(matches!(compiled.materialization, Materialization::Table));
    }

    #[test]
    fn test_ref_with_double_quotes() {
        // Path form uses identifiers, no quoting variants — test subdirectory path
        let sql = r#"SELECT * FROM smelt.model_a"#;

        let model = ModelFile {
            name: "test".to_string(),
            path: "models/test.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(compiled.sql.contains("FROM main.model_a"));
        assert!(!compiled.sql.contains("smelt.models"));
    }

    #[test]
    fn test_ref_with_whitespace() {
        // Whitespace inside refs was a legacy smelt.ref() concern; path form
        // has no arg-list parens. Test a path ref with a nested subdirectory segment.
        let sql = r#"SELECT * FROM smelt.model_a"#;

        let model = ModelFile {
            name: "test".to_string(),
            path: "models/test.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(compiled.sql.contains("FROM main.model_a"));
        assert!(!compiled.sql.contains("smelt.models"));
    }

    #[test]
    fn test_multiple_refs_same_model() {
        let sql = r#"
SELECT a.id, b.id
FROM smelt.model_a a
JOIN smelt.model_a b ON a.parent_id = b.id
"#;

        let model = ModelFile {
            name: "test".to_string(),
            path: "models/test.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        // Both instances should be replaced
        assert_eq!(compiled.sql.matches("main.model_a").count(), 2);
        assert!(!compiled.sql.contains("smelt.models"));
    }

    #[test]
    fn test_refs_preserve_formatting() {
        let sql = r#"
SELECT
    user_id,
    COUNT(*) as count
FROM smelt.events
WHERE event_type = 'click'
"#;

        let model = ModelFile {
            name: "test".to_string(),
            path: "models/test.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        // Verify formatting is preserved (newlines, indentation)
        assert!(compiled.sql.contains("SELECT\n    user_id,"));
        assert!(compiled.sql.contains("FROM main.events"));
        assert!(compiled.sql.contains("WHERE event_type = 'click'"));
        assert!(!compiled.sql.contains("smelt.models"));
    }

    // ===== Ephemeral model tests =====

    #[test]
    fn test_ephemeral_simple_cte_inlining() {
        // Ephemeral model: staging_users
        let ephemeral_sql = "SELECT id, name FROM raw_users WHERE active = true";

        // Downstream model references the ephemeral
        let sql = "SELECT * FROM smelt.staging_users";
        let model = ModelFile {
            name: "final_users".to_string(),
            path: "models/final_users.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let caps = BackendCapabilities::duckdb();
        let resolver = EphemeralResolver::new(
            &[("staging_users".to_string(), ephemeral_sql.to_string())],
            &SqlDialect::DuckDB,
            &caps,
            "main",
        );

        let compiled = compiler
            .compile_with_ephemerals(&model, "main", &resolver)
            .unwrap();

        assert!(compiled.sql.contains("__smelt_staging_users"));
        assert!(compiled.sql.contains("WITH"));
        assert!(!compiled.sql.contains("smelt.models"));
        assert!(!compiled.sql.contains("main.staging_users"));
    }

    #[test]
    fn test_ephemeral_transitive_deps() {
        // C (ephemeral) -> B (ephemeral) -> A (table)
        let c_sql = "SELECT * FROM raw_data";
        let b_sql = "SELECT * FROM smelt.c";

        let sql = "SELECT * FROM smelt.b";
        let model = ModelFile {
            name: "a".to_string(),
            path: "models/a.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let caps = BackendCapabilities::duckdb();
        let resolver = EphemeralResolver::new(
            &[
                ("c".to_string(), c_sql.to_string()),
                ("b".to_string(), b_sql.to_string()),
            ],
            &SqlDialect::DuckDB,
            &caps,
            "main",
        );

        let compiled = compiler
            .compile_with_ephemerals(&model, "main", &resolver)
            .unwrap();

        // Both C and B should be in the CTE list
        assert!(compiled.sql.contains("__smelt_c"));
        assert!(compiled.sql.contains("__smelt_b"));
        // C should come before B (topological order)
        let c_pos = compiled.sql.find("__smelt_c").unwrap();
        let b_pos = compiled.sql.find("__smelt_b").unwrap();
        assert!(c_pos < b_pos, "C should appear before B in CTEs");
    }

    #[test]
    fn test_ephemeral_mixed_refs() {
        // staging (ephemeral), regular_model (table)
        let staging_sql = "SELECT * FROM raw_data";

        let sql = "SELECT * FROM smelt.staging JOIN smelt.regular_model ON 1=1";
        let model = ModelFile {
            name: "final".to_string(),
            path: "models/final.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let caps = BackendCapabilities::duckdb();
        let resolver = EphemeralResolver::new(
            &[("staging".to_string(), staging_sql.to_string())],
            &SqlDialect::DuckDB,
            &caps,
            "main",
        );

        let compiled = compiler
            .compile_with_ephemerals(&model, "main", &resolver)
            .unwrap();

        // Ephemeral → CTE name, non-ephemeral → schema.table
        // Ephemeral → CTE name, non-ephemeral → schema.table
        assert!(compiled.sql.contains("__smelt_staging"));
        assert!(compiled.sql.contains("main.regular_model"));
    }

    #[test]
    fn test_ephemeral_with_existing_with_clause() {
        let staging_sql = "SELECT * FROM raw_data";

        let sql = "WITH my_cte AS (SELECT 1 as x) SELECT * FROM smelt.staging JOIN my_cte ON 1=1";
        let model = ModelFile {
            name: "final".to_string(),
            path: "models/final.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let caps = BackendCapabilities::duckdb();
        let resolver = EphemeralResolver::new(
            &[("staging".to_string(), staging_sql.to_string())],
            &SqlDialect::DuckDB,
            &caps,
            "main",
        );

        let compiled = compiler
            .compile_with_ephemerals(&model, "main", &resolver)
            .unwrap();

        // Should have a single WITH clause with both CTEs
        let with_count = compiled.sql.matches("WITH ").count();
        assert_eq!(with_count, 1, "Should have exactly one WITH clause");
        assert!(
            compiled.sql.contains("__smelt_staging"),
            "compiled: {}",
            compiled.sql
        );
        assert!(compiled.sql.contains("my_cte"));
    }

    #[test]
    fn test_prepend_ephemeral_ctes_no_existing_with() {
        let cte_list = vec![
            ("__smelt_a".to_string(), "SELECT 1 as x".to_string()),
            (
                "__smelt_b".to_string(),
                "SELECT * FROM __smelt_a".to_string(),
            ),
        ];
        let sql = "SELECT * FROM __smelt_b";
        let result = prepend_ephemeral_ctes(sql, &cte_list);

        assert!(result.starts_with("WITH"));
        assert!(result.contains("__smelt_a AS"));
        assert!(result.contains("__smelt_b AS"));
        assert!(result.contains("SELECT * FROM __smelt_b"));
    }

    #[test]
    fn test_prepend_ephemeral_ctes_with_existing_with() {
        let cte_list = vec![("__smelt_staging".to_string(), "SELECT 1 as x".to_string())];
        let sql = "WITH my_cte AS (SELECT 2 as y) SELECT * FROM __smelt_staging JOIN my_cte";
        let result = prepend_ephemeral_ctes(sql, &cte_list);

        let with_count = result.matches("WITH ").count();
        assert_eq!(with_count, 1, "Should merge into single WITH");
        assert!(result.contains("__smelt_staging AS"));
        assert!(result.contains("my_cte AS"));
    }

    #[test]
    fn test_extract_cte_parts() {
        let sql = "WITH a AS (SELECT 1), b AS (SELECT 2 FROM a) SELECT * FROM b";
        let parts = extract_cte_parts(sql);

        assert_eq!(parts.ctes.len(), 2);
        assert_eq!(parts.ctes[0].0, "a");
        assert_eq!(parts.ctes[0].1, "SELECT 1");
        assert_eq!(parts.ctes[1].0, "b");
        assert_eq!(parts.ctes[1].1, "SELECT 2 FROM a");
        assert_eq!(parts.main_body, "SELECT * FROM b");
    }

    #[test]
    fn test_rename_table_references() {
        let sql = "SELECT * FROM cleaned WHERE cleaned.id > 0";
        let result = rename_table_references(sql, "cleaned", "__smelt_model__cleaned");
        assert!(result.contains("__smelt_model__cleaned"));
        assert!(!result.contains(" cleaned"));
    }

    #[test]
    fn test_case_expression_with_alias_no_question_marks() {
        let sql = "SELECT CASE WHEN x > 0 THEN 'high' ELSE 'low' END AS label FROM t";

        let model = ModelFile {
            name: "case_test".to_string(),
            path: "models/case_test.sql".into(),
            content: sql.to_string(),
            refs: vec![],
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        // Should NOT contain question marks in the output
        assert!(
            !compiled.sql.contains("CAST(? AS"),
            "CASE expression should not produce CAST(? AS ...): {}",
            compiled.sql
        );
        assert!(
            !compiled.sql.contains("AS ?"),
            "CASE expression should not produce ... AS ?: {}",
            compiled.sql
        );
        // Should contain the alias 'label'
        assert!(
            compiled.sql.contains("label"),
            "Should preserve the 'label' alias: {}",
            compiled.sql
        );
    }

    #[test]
    fn test_case_expression_without_alias_no_question_marks() {
        // CASE without explicit alias — should produce a valid name, not '?'
        let sql = "SELECT x, CASE WHEN x > 0 THEN 'high' ELSE 'low' END FROM t";

        let model = ModelFile {
            name: "case_test2".to_string(),
            path: "models/case_test2.sql".into(),
            content: sql.to_string(),
            refs: vec![],
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(
            !compiled.sql.contains("CAST(? AS"),
            "CASE without alias should not produce CAST(? AS ...): {}",
            compiled.sql
        );
        assert!(
            !compiled.sql.contains("AS ?"),
            "CASE without alias should not produce ... AS ?: {}",
            compiled.sql
        );
    }

    #[test]
    fn test_join_type_inference_no_wrong_casts() {
        // When a model JOINs source + seed, the type wrapper should not apply wrong types
        let sql = r#"SELECT
    p.product_id,
    p.product_name,
    ch.category_name,
    p.unit_price_cents / 100.0 AS unit_price,
    CASE WHEN p.is_digital THEN 'Digital' ELSE 'Physical' END AS product_type
FROM raw.products AS p
LEFT JOIN main.category_hierarchy AS ch ON p.category_code = ch.category_code"#;

        let model = ModelFile {
            name: "stg_products".to_string(),
            path: "models/staging/stg_products.sql".into(),
            content: sql.to_string(),
            refs: vec![],
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        // product_name is a VARCHAR column — should NOT be cast as DOUBLE
        assert!(
            !compiled.sql.contains("CAST(product_name AS DOUBLE)"),
            "product_name should not be cast as DOUBLE: {}",
            compiled.sql
        );
        // product_id is INTEGER — should NOT be cast as DECIMAL(11,10)
        assert!(
            !compiled.sql.contains("DECIMAL(11,10)"),
            "product_id should not get wrong DECIMAL precision: {}",
            compiled.sql
        );
    }

    #[test]
    fn test_case_in_aggregate_no_question_marks() {
        // COUNT(CASE WHEN ... THEN 1 END) — common funnel pattern
        let sql = "SELECT COUNT(CASE WHEN event_type = 'purchase' THEN 1 END) AS purchases FROM t GROUP BY x";

        let model = ModelFile {
            name: "case_agg_test".to_string(),
            path: "models/case_agg_test.sql".into(),
            content: sql.to_string(),
            refs: vec![],
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(
            !compiled.sql.contains("CAST(? AS"),
            "CASE in aggregate should not produce CAST(? AS ...): {}",
            compiled.sql
        );
        assert!(
            compiled.sql.contains("purchases"),
            "Should preserve the 'purchases' alias: {}",
            compiled.sql
        );
    }

    // ===== Contract: ephemeral models and smelt.define functions have no DB name =====

    /// Verify that ephemeral models go through the CTE-inlining path and never
    /// produce a materialised table reference (`main.<name>`).
    ///
    /// This documents the contract that `default_db_name` MUST NOT be called for
    /// ephemeral entities.  If an ephemeral model were accidentally passed through
    /// `default_db_name`, the downstream model's compiled SQL would contain a bare
    /// table reference (`main.staging_users`) instead of the correct CTE alias
    /// (`__smelt_staging_users`).  This test is the TDD anchor for that invariant.
    #[test]
    fn ephemeral_and_define_have_no_db_name() {
        // --- setup: one ephemeral model "staging_users" ---
        let ephemeral_sql = "SELECT id, name FROM raw_users WHERE active = true";

        // Downstream model that references the ephemeral via the new path-ref syntax.
        let downstream_sql = "SELECT * FROM smelt.staging_users";
        let model = ModelFile {
            name: "final_users".to_string(),
            path: "models/final_users.sql".into(),
            content: downstream_sql.to_string(),
            refs: extract_refs_from_sql(downstream_sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("final_users.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());
        let caps = BackendCapabilities::duckdb();
        let resolver = EphemeralResolver::new(
            &[("staging_users".to_string(), ephemeral_sql.to_string())],
            &SqlDialect::DuckDB,
            &caps,
            "main",
        );

        let compiled = compiler
            .compile_with_ephemerals(&model, "main", &resolver)
            .unwrap();

        // The ephemeral's content is inlined as a CTE with the `__smelt_` prefix.
        assert!(
            compiled.sql.contains("__smelt_staging_users"),
            "Ephemeral must be inlined as `__smelt_staging_users` CTE, not a table: {}",
            compiled.sql
        );
        // No materialised table reference — `default_db_name` was NOT invoked for the ephemeral.
        assert!(
            !compiled.sql.contains("main.staging_users"),
            "Ephemeral must NOT produce a materialised table reference `main.staging_users`: {}",
            compiled.sql
        );
        // Confirm the CTE wrapper is present.
        assert!(
            compiled.sql.contains("WITH"),
            "Ephemeral inlining must produce a WITH clause: {}",
            compiled.sql
        );
    }

    // ===== Named-argument substitution tests =====

    /// Helper: build a params vec with no defaults for use in unit tests.
    fn params_no_defaults(names: &[&str]) -> Vec<(String, Option<String>)> {
        names.iter().map(|n| (n.to_string(), None)).collect()
    }

    /// Helper: build a params vec where the last param has a default value.
    fn params_with_last_default(names: &[&str], default: &str) -> Vec<(String, Option<String>)> {
        let mut v: Vec<(String, Option<String>)> =
            names.iter().map(|n| (n.to_string(), None)).collect();
        if let Some(last) = v.last_mut() {
            last.1 = Some(default.to_string());
        }
        v
    }

    /// Smoke test: calling `substitute_params_with_named` with a mix of
    /// positional + named args produces the same substituted body as a pure
    /// positional call.
    #[test]
    fn substitute_named_args_binds_by_param_name() {
        // Function: safe_divide(numerator, denominator)
        // Body: CASE WHEN denominator = 0 THEN NULL ELSE numerator / denominator END
        let body = "CASE WHEN denominator = 0 THEN NULL ELSE numerator / denominator END";
        let params = params_no_defaults(&["numerator", "denominator"]);

        // Positional call: safe_divide(revenue, cost)
        let positional_args = vec!["revenue".to_string(), "cost".to_string()];
        let positional_result = substitute_params_with_named(body, &params, &positional_args, &[]);

        // Named call: safe_divide(numerator => revenue, denominator => cost)
        let named_args = vec![
            ("numerator".to_string(), "revenue".to_string()),
            ("denominator".to_string(), "cost".to_string()),
        ];
        let named_result = substitute_params_with_named(body, &params, &[], &named_args);

        assert_eq!(
            positional_result, named_result,
            "named-arg and positional calls should produce identical substituted bodies"
        );
        // Both must actually substitute.
        assert!(
            named_result.contains("revenue") && named_result.contains("cost"),
            "expected revenue/cost in substituted body, got: {named_result}"
        );
        assert!(
            !named_result.contains("numerator") && !named_result.contains("denominator"),
            "parameter names must be replaced, got: {named_result}"
        );
    }

    /// Named arguments may appear in any order; binding by name must reorder them
    /// to match the declaration order.
    #[test]
    fn substitute_named_args_reorders_independent_of_call_order() {
        // 3-parameter function with params [a, b, c]
        let body = "a + b * c";
        let params = params_no_defaults(&["a", "b", "c"]);

        // Call with args in reverse order: (c => 'C', a => 'A', b => 'B')
        let named_args = vec![
            ("c".to_string(), "'C'".to_string()),
            ("a".to_string(), "'A'".to_string()),
            ("b".to_string(), "'B'".to_string()),
        ];

        let result = substitute_params_with_named(body, &params, &[], &named_args);

        assert_eq!(
            result, "'A' + 'B' * 'C'",
            "each param must be replaced by its named arg regardless of call order, got: {result}"
        );
    }

    /// Positional args fill the first slots; subsequent named args fill the rest.
    #[test]
    fn substitute_named_args_mixes_positional_then_named() {
        let body = "FUNC(x, y)";
        let params = params_no_defaults(&["x", "y"]);

        // Call: (pos_x, y => named_y)
        let positional = vec!["pos_x".to_string()];
        let named = vec![("y".to_string(), "named_y".to_string())];

        let result = substitute_params_with_named(body, &params, &positional, &named);

        assert_eq!(
            result, "FUNC(pos_x, named_y)",
            "positional fills first slot, named fills second, got: {result}"
        );
    }

    /// An unknown named argument must not cause a panic or silent miscompile;
    /// the unfilled slot keeps the original parameter name so the downstream
    /// SQL engine surfaces a clear error (the type-checker has already rejected
    /// this via `UnknownPassingParameter`).
    #[test]
    fn substitute_named_args_unknown_name_passes_through() {
        let body = "numerator / denominator";
        let params = params_no_defaults(&["numerator", "denominator"]);

        // Slot for `numerator` is filled; slot for `denominator` is unknown.
        let named_args = vec![
            ("numerator".to_string(), "revenue".to_string()),
            ("unknown_param".to_string(), "value".to_string()),
        ];

        let result = substitute_params_with_named(body, &params, &[], &named_args);

        // `numerator` must be replaced; `denominator` must remain (unfilled slot).
        assert!(
            result.contains("revenue"),
            "filled slot must be replaced, got: {result}"
        );
        assert!(
            result.contains("denominator"),
            "unfilled slot must remain as the param name so downstream SQL fails clearly, got: {result}"
        );
    }

    /// A parameter with a declared default value must use the default when
    /// neither positional nor named arg is supplied at the call site.
    #[test]
    fn substitute_named_args_uses_default_for_omitted_param() {
        // Function: sessionize(source, gap = INTERVAL '30 minutes')
        let body = "SELECT *, ts - LAG(ts) OVER (...) > gap FROM source";
        let params = params_with_last_default(&["source", "gap"], "INTERVAL '30 minutes'");

        // Call omitting `gap` — should use default.
        let positional = vec!["my_table".to_string()];
        let result = substitute_params_with_named(body, &params, &positional, &[]);

        assert!(
            result.contains("my_table"),
            "source must be substituted, got: {result}"
        );
        assert!(
            result.contains("INTERVAL '30 minutes'"),
            "default for gap must be used when omitted, got: {result}"
        );
        assert!(
            !result.contains(" gap"),
            "gap param name must be replaced by its default, got: {result}"
        );
    }
}
