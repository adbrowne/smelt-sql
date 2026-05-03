use crate::config::{BackendType, Config, Materialization, Target};
use crate::discovery::ModelFile;
use crate::errors::{extract_snippet, text_range_to_line_col, CliError};
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

/// Type for the pre-resolved function-body map: fn_name → (param_names, body_sql).
pub type FnBodyMap = HashMap<String, (Vec<String>, String)>;

/// Substitute `param_names` with `arg_sqls` in a function body SQL string.
///
/// Replaces whole-word occurrences of each parameter name with the
/// corresponding argument SQL, skipping inside single-quoted strings.
/// Named args (`key => value`) are not yet supported here; only positional.
fn substitute_params(body: &str, param_names: &[String], arg_sqls: &[String]) -> String {
    let mut result = body.to_string();
    for (param, arg) in param_names.iter().zip(arg_sqls.iter()) {
        result = replace_identifier(&result, param, arg);
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
}

impl UpstreamSchemas {
    /// Build an `UpstreamSchemas` from a populated Salsa `Database` and the
    /// list of model files registered in it. The CLI passes this into every
    /// `SqlCompiler` so `apply_type_casts` can resolve `smelt.ref()` columns
    /// without going through Salsa itself (the batch compiler is pure).
    ///
    /// `models` is the same list that was passed to `init_db` — we use it to
    /// know which paths to query, and to recover each model's user-facing name.
    pub fn from_database(
        db: &smelt_db::Database,
        project_dir: &std::path::Path,
        models: &[crate::discovery::ModelFile],
    ) -> Self {
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
        let paths = smelt_core::Config::load(project_dir)
            .map(|c| c.paths)
            .unwrap_or_else(|_| vec!["models".to_string()]);
        let mut seed_schemas: HashMap<String, Vec<(String, TypedColumn)>> = HashMap::new();
        for seed in smelt_core::discover_seed_infos(project_dir, &paths) {
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
            seed_schemas.insert(seed.name, cols);
        }

        let sources = SourcesConfig::load(project_dir).unwrap_or_default();

        Self {
            models: model_schemas,
            seeds: seed_schemas,
            sources,
        }
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
                move |fn_name: &str, positional: Vec<String>, _named: Vec<(String, String)>| {
                    let (param_names, body_sql) = bodies.get(fn_name)?;
                    Some(substitute_params(body_sql, param_names, &positional))
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
                              _named: Vec<(String, String)>| {
                            let fn_name = segs.last()?;
                            let (param_names, body_sql) = bodies.get(fn_name)?;
                            Some(substitute_params(body_sql, param_names, &positional))
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

    /// Build a `SmeltPathRefResolver` for a specific `schema` string, wiring
    /// `smelt.models.*` / `smelt.sources.*` / `smelt.seeds.*` to the
    /// appropriate backend SQL expressions.
    ///
    /// - `["models", name]` → `schema.name` (or cross-engine expression)
    /// - `["seeds", name...]` → `schema.<name_joined_with_underscores>`
    /// - `["sources", src_name, table_name]` → `src_name.table_name`
    ///   (matching the legacy `smelt.sources.src.tbl` resolution)
    ///
    /// Paths not matching any known namespace return `None`, leaving the
    /// node verbatim — forward-compatible with new namespaces.
    fn make_path_ref_resolver(&self, schema: &str) -> SmeltPathRefResolver<'static> {
        self.make_path_ref_resolver_with_ephemerals(schema, &HashSet::new())
    }

    /// Like `make_path_ref_resolver` but emits `__smelt_{name}` for any model
    /// whose leaf name appears in `ephemeral_names`.  Used by
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
        let ephemerals = ephemeral_names.clone();

        Box::new(move |segs: &[String]| {
            match segs {
                // smelt.models.<path...>.<name> — subdirectory models use the
                // leaf segment as the physical table name.
                [ns, rest @ ..] if ns == "models" && !rest.is_empty() => {
                    let name = rest.last().expect("rest non-empty");
                    // Ephemeral models resolve to their CTE alias.
                    if ephemerals.contains(name) {
                        return Some(format!("__smelt_{}", name));
                    }
                    if let Some(parquet_expr) = cross_engine_refs.get(name) {
                        Some(parquet_expr.clone())
                    } else {
                        Some(format!("{}.{}", schema, name))
                    }
                }
                // smelt.seeds.<name...> — join path segments with '_'
                [ns, rest @ ..] if ns == "seeds" && !rest.is_empty() => {
                    let table_name = rest.join("_");
                    Some(format!("{}.{}", schema, table_name))
                }
                // smelt.sources.<source_name>.<table_name>
                [ns, source_name, table_name] if ns == "sources" => {
                    // Apply any `identifier` override from sources.yml.
                    let emit_name = sources
                        .sources
                        .iter()
                        .find(|s| s.name == *source_name)
                        .and_then(|src| src.tables.iter().find(|t| t.name == *table_name))
                        .and_then(|tbl| tbl.identifier.as_deref())
                        .unwrap_or(table_name.as_str())
                        .to_string();
                    Some(format!("{}.{}", source_name, emit_name))
                }
                _ => None,
            }
        })
    }

    /// Compile a model's SQL by replacing smelt.ref() calls with table references
    pub fn compile(&self, model: &ModelFile, schema: &str) -> Result<CompiledModel> {
        // ERROR if any named parameters detected
        for ref_info in &model.refs {
            if ref_info.has_named_params {
                let (line, col) = text_range_to_line_col(&model.content, ref_info.range);
                let snippet = extract_snippet(&model.content, ref_info.range, 0);

                return Err(CliError::NamedParametersNotSupported {
                    model: model.name.clone(),
                    file: model.path.clone(),
                    line,
                    col,
                    snippet,
                }
                .into());
            }
        }

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
            name: model.name.clone(),
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
        // so SUM/COUNT/AVG over `smelt.models.upstream.col` resolve correctly.
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
            name: model.name.clone(),
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
            // Use ephemeral-aware resolver so smelt.models.<ephemeral> → __smelt_<name>
            smelt_path_ref: Some(
                self.make_path_ref_resolver_with_ephemerals(schema, &resolver.ephemeral_names),
            ),
            smelt_path_call: path_call_expander,
        };
        let compiled_sql = smelt_dialect::print(&parse.syntax(), &ctx);
        let compiled_sql = self.apply_type_casts(&compiled_sql);

        // Collect which ephemeral models this model references
        let referenced: Vec<&str> = model
            .refs
            .iter()
            .filter(|r| resolver.ephemeral_names.contains(&r.model_name))
            .map(|r| r.model_name.as_str())
            .collect();

        let final_sql = if referenced.is_empty() {
            compiled_sql
        } else {
            let cte_list = resolver.get_cte_list(&referenced);
            prepend_ephemeral_ctes(&compiled_sql, &cte_list)
        };

        let materialization = self.config.get_materialization_with_metadata(
            &model.name,
            model.metadata.as_ref().map(|b| b.as_ref()),
        );

        Ok(CompiledModel {
            name: model.name.clone(),
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

        // Build a path-ref resolver that maps smelt.models.<name> to either
        // __smelt_<name> (if ephemeral) or schema.<name> (if physical).
        let ephemerals_owned: HashSet<String> = ephemeral_names.clone();
        let schema_owned = schema.to_string();
        let path_ref_resolver: SmeltPathRefResolver<'static> =
            Box::new(move |segs: &[String]| match segs {
                [ns, rest @ ..] if ns == "models" && !rest.is_empty() => {
                    let name = rest.last().expect("rest non-empty");
                    if ephemerals_owned.contains(name) {
                        Some(format!("__smelt_{}", name))
                    } else {
                        Some(format!("{}.{}", schema_owned, name))
                    }
                }
                [ns, rest @ ..] if ns == "seeds" && !rest.is_empty() => {
                    let table_name = rest.join("_");
                    Some(format!("{}.{}", schema_owned, table_name))
                }
                [ns, source_name, table_name] if ns == "sources" => {
                    Some(format!("{}.{}", source_name, table_name))
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
        // Check for named params (same as compile)
        for ref_info in &model.refs {
            if ref_info.has_named_params {
                let (line, col) = text_range_to_line_col(&model.content, ref_info.range);
                let snippet = extract_snippet(&model.content, ref_info.range, 0);
                return Err(CliError::NamedParametersNotSupported {
                    model: model.name.clone(),
                    file: model.path.clone(),
                    line,
                    col,
                    snippet,
                }
                .into());
            }
        }

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
            // Use ephemeral-aware resolver so smelt.models.<ephemeral> → __smelt_<name>
            smelt_path_ref: Some(
                self.make_path_ref_resolver_with_ephemerals(schema, &resolver.ephemeral_names),
            ),
            smelt_path_call: path_call_expander,
        };
        let compiled_sql = smelt_dialect::print(&parse.syntax(), &ctx);
        let compiled_sql = self.apply_type_casts(&compiled_sql);

        // Collect which ephemeral models this model references.
        // model.refs carries the leaf segment as model_name, which matches
        // resolver.ephemeral_names (also leaf-segment keyed).
        let referenced: Vec<&str> = model
            .refs
            .iter()
            .filter(|r| resolver.ephemeral_names.contains(&r.model_name))
            .map(|r| r.model_name.as_str())
            .collect();

        // Prepend ephemeral CTEs if any
        let final_sql = if referenced.is_empty() {
            compiled_sql
        } else {
            let cte_list = resolver.get_cte_list(&referenced);
            prepend_ephemeral_ctes(&compiled_sql, &cte_list)
        };

        let materialization = self.config.get_materialization_with_metadata(
            &model.name,
            model.metadata.as_ref().map(|b| b.as_ref()),
        );

        Ok(CompiledModel {
            name: model.name.clone(),
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

/// Walk every file in `workspace` and extract `smelt.define` bodies as a
/// [`FnBodyMap`] keyed by leaf function name.
///
/// The `(param_names, body_sql)` payload is what `SqlCompiler`'s
/// `smelt.fn.*` expander substitutes into call sites at print time. Body
/// extraction uses the parser's `DEFINE_BODY` `text_range`, which spans the
/// surrounding parens (e.g. `(CASE WHEN ... END)`); substituting a
/// parenthesised expression at the call site preserves precedence.
///
/// Pure: takes an immutable `&Database` and returns plain data. The
/// orchestration layer (`commands/run.rs`, `commands/backbuild.rs`) is the
/// only place that calls into Salsa to build the inputs for this helper, per
/// the pure-function rule in CLAUDE.md.
///
/// On a workspace-level duplicate function name (already a separate
/// diagnostic via `workspace_function_diagnostics`), later entries silently
/// overwrite earlier ones in iteration order. "First declaration wins" is a
/// diagnostic concern, not a runtime one.
///
/// Models without `smelt.define` declarations contribute zero entries; an
/// empty `functions/` directory yields an empty map.
pub fn build_fn_body_map(db: &smelt_db::Database, workspace: smelt_db::Workspace) -> FnBodyMap {
    let mut out: FnBodyMap = HashMap::new();
    for file in workspace.files(db).iter().copied() {
        let parse = smelt_db::parse_file(db, file);
        let Some(ast) = File::cast(parse.syntax()) else {
            continue;
        };
        // `parse_file` strips frontmatter while preserving byte offsets, so
        // text-range offsets index into either the raw or stripped text
        // identically. We use the raw `file.text(db)` here so the extracted
        // body is what users see in their source files.
        let text = file.text(db);
        for define in ast.defines() {
            let Some(name) = define.name() else { continue };
            let Some(body) = define.body() else { continue };
            let range = body.syntax().text_range();
            let start = usize::from(range.start());
            let end = usize::from(range.end());
            if end > text.len() || start > end {
                continue;
            }
            let body_sql = text[start..end].to_string();
            let params: Vec<String> = define
                .param_list()
                .map(|pl| pl.params().filter_map(|p| p.name()).collect())
                .unwrap_or_default();
            out.insert(name, (params, body_sql));
        }
    }
    out
}

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
FROM smelt.models.raw_events
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
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(compiled.sql.contains("FROM main.raw_events"));
        assert!(!compiled.sql.contains("smelt.models.raw_events"));
    }

    #[test]
    fn test_multiple_refs() {
        let sql = r#"
SELECT a.user_id, b.session_id
FROM smelt.models.model_a a
JOIN smelt.models.model_b b ON a.id = b.id
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
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(compiled.sql.contains("FROM main.model_a a"));
        assert!(compiled.sql.contains("JOIN main.model_b b"));
        assert!(!compiled.sql.contains("smelt.models"));
    }

    #[test]
    fn test_named_params_error() {
        // After Phase 4 the legacy `smelt.ref('x', filter => ...)` parse error means
        // named-param refs can no longer come from model SQL.  Test the compiler
        // guard directly by constructing a RefInfo with has_named_params=true.
        use smelt_core::refs::SmeltRef;
        let sql = "SELECT user_id FROM smelt.models.raw_events";

        let named_ref = RefInfo {
            model_name: "raw_events".to_string(),
            has_named_params: true,
            range: rowan::TextRange::new(0.into(), 1.into()),
            smelt_ref: SmeltRef::Path(vec!["models".to_string(), "raw_events".to_string()]),
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
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let result = compiler.compile(&model, "main");
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("named parameters"));
        assert!(err_msg.contains("not yet supported"));
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
        };

        let mut config = make_test_config();
        config.models.insert(
            "test_model".to_string(),
            ModelConfig {
                materialization: Some(Materialization::Table),
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
        let sql = r#"SELECT * FROM smelt.models.model_a"#;

        let model = ModelFile {
            name: "test".to_string(),
            path: "models/test.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
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
        let sql = r#"SELECT * FROM smelt.models.model_a"#;

        let model = ModelFile {
            name: "test".to_string(),
            path: "models/test.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
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
FROM smelt.models.model_a a
JOIN smelt.models.model_a b ON a.parent_id = b.id
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
FROM smelt.models.events
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
        let sql = "SELECT * FROM smelt.models.staging_users";
        let model = ModelFile {
            name: "final_users".to_string(),
            path: "models/final_users.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
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
        let b_sql = "SELECT * FROM smelt.models.c";

        let sql = "SELECT * FROM smelt.models.b";
        let model = ModelFile {
            name: "a".to_string(),
            path: "models/a.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
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

        let sql = "SELECT * FROM smelt.models.staging JOIN smelt.models.regular_model ON 1=1";
        let model = ModelFile {
            name: "final".to_string(),
            path: "models/final.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
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

        let sql =
            "WITH my_cte AS (SELECT 1 as x) SELECT * FROM smelt.models.staging JOIN my_cte ON 1=1";
        let model = ModelFile {
            name: "final".to_string(),
            path: "models/final.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: crate::discovery::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
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
}
