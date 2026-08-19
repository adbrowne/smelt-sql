use anyhow::Result;
use smelt_core::config::{BackendType, Config, Materialization, RefreshStrategy, Target};
use smelt_core::{ModelFile, SourcesConfig};
use smelt_db::type_inference::infer_select_column_types;
use smelt_db::{
    add_source_info_to_type_context, build_type_context, StaticRefSchemaProvider, TypeContext,
};
use smelt_dialect::{
    wrap_with_type_casts, AsStructEmitter, BackendCapabilities, PrintContext, SmeltFnExpander,
    SmeltPathCallExpander, SmeltPathRefResolver, SqlDialect,
};
use smelt_parser::ast::{File, SelectStmt};
use smelt_types::{DataType, TypedColumn};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::fn_bodies::FnBodyMap;

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
/// * `horizon_ceiling` — The model's declared `horizon_ceiling:` warning ceiling, if any.
///   Never narrows or widens the derived bound (see `docs/specs/incremental_models.md`
///   §"Windowed maintenance and the horizon") — it only licenses a warning message in the
///   returned `Vec<String>` when a source's derived reach exceeds it.
///
/// Returns the bound map (used exactly as before) alongside any horizon-ceiling warnings
/// collected while building it, so callers can surface them (e.g. via `tracing::warn!`)
/// without a second walk over the sources.
pub fn build_source_bound_map(
    model_sql: &str,
    dep_timeseries: &HashMap<String, (Vec<String>, String)>,
    horizon_ceiling: Option<&smelt_core::config::DataLatency>,
) -> (
    HashMap<String, crate::transformer::SourceBound>,
    Vec<String>,
) {
    use smelt_logical::analysis::horizon_ceiling::check_horizon_ceiling;
    use smelt_planner::analysis::source_bounds::{
        derive_and_classify_bounds, BoundContext, BoundResult,
    };

    if dep_timeseries.is_empty() {
        return (HashMap::new(), Vec::new());
    }

    // Build BoundContext: dep_name → partition_col
    let mut ctx = BoundContext::new();
    for (dep_name, (_segs, partition_col)) in dep_timeseries {
        ctx.add_source(dep_name, partition_col);
    }

    // The single bound-derivation orchestration entry point (also used by
    // `rules::incremental::derive_model_source_bounds` on the planner side) —
    // one `derive_model_bounds` walk feeds both consumers.
    let raw_bounds = derive_and_classify_bounds(model_sql, &ctx);

    let mut result = HashMap::new();
    let mut warnings = Vec::new();
    for (dep_name, (segs, partition_col)) in dep_timeseries {
        let bound_result = raw_bounds.get(dep_name).map(|(_, bound)| bound.clone());

        // The horizon ceiling is a warning-only comparison against the derived
        // reach — it never changes `before_secs`/`after_secs` below. Compare
        // before the match consumes `bound_result`.
        if let (Some(bound), Some(ceiling)) = (bound_result.as_ref(), horizon_ceiling) {
            if let Some(msg) = check_horizon_ceiling(bound, ceiling.seconds) {
                warnings.push(format!("source '{}': {}", dep_name, msg));
            }
        }

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

    (result, warnings)
}

/// Derive the F1 bound-based [`smelt_planner::BatchSafety`] classification for a
/// model's backfill chunk-size decision.
///
/// This is the batched-local consumer of the same `BoundContext` /
/// `derive_and_classify_bounds` walk `build_source_bound_map` uses — there is
/// no second, independent bound derivation. `Err` means a source's bound is
/// `NotDerivable` (fail-closed, `incremental_shapes.md` §"Partition-grain constraints" #10): the
/// caller must surface this as a hard refusal, never approximate a chunk
/// shape from it.
///
/// A model with no dependency timeseries info at all (no upstream ref
/// carries `timeseries:`) has no source to bound against, so it is
/// `FullyBatchSafe` — there is nothing to chunk around.
pub fn batch_safety_for_model(
    model_sql: &str,
    dep_timeseries: &HashMap<String, (Vec<String>, String)>,
) -> Result<smelt_planner::BatchSafety, String> {
    use smelt_planner::analysis::source_bounds::{derive_and_classify_bounds, BoundContext};
    use smelt_planner::{batch_safety_from_bounds, BoundResult};

    if dep_timeseries.is_empty() {
        return Ok(smelt_planner::BatchSafety::FullyBatchSafe);
    }

    let mut ctx = BoundContext::new();
    for (dep_name, (_segs, partition_col)) in dep_timeseries {
        ctx.add_source(dep_name, partition_col);
    }

    let raw_bounds = derive_and_classify_bounds(model_sql, &ctx);
    let bounds: HashMap<String, BoundResult> = raw_bounds
        .into_iter()
        .map(|(name, (_, bound))| (name, bound))
        .collect();

    batch_safety_from_bounds(&bounds)
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
pub(crate) fn bind_named_args(
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
        // When the arg is a qualified name (contains a dot, e.g. `main.orders`)
        // and the param appears as a FROM/JOIN table reference in the body, naively
        // text-replacing the param would produce `main.orders.*` — a multi-part
        // wildcard rejected by DuckDB. Instead, alias the argument at the FROM/JOIN
        // position (`FROM main.orders AS source`) and leave all other occurrences of
        // the param unchanged so `source.*` and `source.col` resolve through the alias.
        result = if arg.contains('.') && has_from_table_reference(&result, param_name) {
            replace_from_with_alias(&result, param_name, arg)
        } else {
            replace_identifier(&result, param_name, arg)
        };
    }
    result
}

/// Returns `true` if `param` appears as a whole-word identifier immediately
/// after a `FROM` or `*JOIN` keyword in `body` (case-insensitive for keywords,
/// case-sensitive for `param`). Used to detect `TableExpr` parameters — they
/// always appear as FROM-clause table references; `Expr<T>` params never do.
fn has_from_table_reference(body: &str, param: &str) -> bool {
    if param.is_empty() || !body.contains(param) {
        return false;
    }
    let chars: Vec<char> = body.chars().collect();
    let param_chars: Vec<char> = param.chars().collect();
    let n = chars.len();
    let m = param_chars.len();
    let mut i = 0;
    let mut in_string = false;

    while i < n {
        if chars[i] == '\'' {
            if in_string {
                if i + 1 < n && chars[i + 1] == '\'' {
                    i += 2;
                    continue;
                }
                in_string = false;
            } else {
                in_string = true;
            }
            i += 1;
            continue;
        }
        if in_string {
            i += 1;
            continue;
        }

        let before_ok = i == 0 || !is_ident_char(chars[i - 1]);
        let slice_matches = i + m <= n
            && chars[i..i + m]
                .iter()
                .zip(param_chars.iter())
                .all(|(a, b)| *a == *b);
        let after_ok = i + m >= n || !is_ident_char(chars[i + m]);

        if before_ok && slice_matches && after_ok {
            // Check if the preceding non-whitespace keyword is FROM or *JOIN.
            let prefix: String = chars[..i].iter().collect();
            let prefix_upper = prefix.trim_end().to_uppercase();
            if prefix_upper.ends_with("FROM") || prefix_upper.ends_with("JOIN") {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Replace `FROM <param>` and `*JOIN <param>` occurrences in `body` with
/// `FROM <arg> AS <param>` / `*JOIN <arg> AS <param>`. All other occurrences
/// of `<param>` are emitted verbatim so they resolve through the alias.
///
/// Used when `arg` is a qualified name (contains `.`) to keep `param.*` and
/// `param.col` references valid — the alias makes them single-part identifiers.
fn replace_from_with_alias(body: &str, param: &str, arg: &str) -> String {
    if param.is_empty() {
        return body.to_string();
    }
    let chars: Vec<char> = body.chars().collect();
    let param_chars: Vec<char> = param.chars().collect();
    let n = chars.len();
    let m = param_chars.len();
    let mut out = String::with_capacity(body.len() + arg.len() + param.len() + 4);
    let mut i = 0;
    let mut in_string = false;

    while i < n {
        if chars[i] == '\'' {
            if in_string {
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

        let before_ok = i == 0 || !is_ident_char(chars[i - 1]);
        let slice_matches = i + m <= n
            && chars[i..i + m]
                .iter()
                .zip(param_chars.iter())
                .all(|(a, b)| *a == *b);
        let after_ok = i + m >= n || !is_ident_char(chars[i + m]);

        if before_ok && slice_matches && after_ok {
            // Check if this occurrence is in a FROM/JOIN position.
            let prefix_upper = out.trim_end().to_uppercase();
            if prefix_upper.ends_with("FROM") || prefix_upper.ends_with("JOIN") {
                // Emit: arg AS param  (the keyword and preceding whitespace are
                // already in `out`; we replace only the param token itself)
                out.push_str(arg);
                out.push_str(" AS ");
                out.push_str(param);
                i += m;
                continue;
            }
            // Not a FROM/JOIN position — leave param verbatim; it resolves
            // through the alias established above.
            out.push_str(param);
            i += m;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
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
    /// The model's output column names, in projection order.
    ///
    /// Empty when they are not statically resolvable — a surviving wildcard,
    /// an unnamed item, or SQL whose top level is not a `SELECT`. Consumers
    /// must treat empty as "unknown", never as "no columns": the
    /// column-scoped `MERGE` on BigQuery needs this list and refuses loudly
    /// without it (`smelt_backend::require_merge_columns`), because GoogleSQL
    /// has no `UPDATE SET *` to fall back to.
    pub output_columns: Vec<String>,
}

/// A model's projection — the ordered list of output columns, derived once
/// from the model's **source** `SelectStmt` (before dialect lowering) plus
/// the `TypeContext` assembled from upstream schemas.
///
/// This is the single owner of "what are this model's output columns and
/// their inferred types": every consumer (the type-cast wrap, the output
/// column list) reads this value rather than re-deriving it from printed,
/// dialect-lowered SQL — see `docs/specs/multi_backend.md`
/// §"Output-schema type conformance".
struct Projection {
    /// `None` when the select list is not statically enumerable — a
    /// wildcard select item is present, or the statement's top level isn't
    /// a plain `SELECT` at all.
    columns: Option<Vec<ProjectionColumn>>,
}

struct ProjectionColumn {
    /// The item's resolved name — an explicit alias, or (today) the same
    /// inferred bare/qualified name `Expression::infer_name` would produce.
    /// `None` when the item has no alias and no inferable name (e.g. a bare
    /// literal); callers decide how to handle that case, since the cast
    /// wrap and the MERGE column list want different fallbacks.
    name: Option<String>,
    typed: TypedColumn,
}

/// Whether `expr`'s inferred name is a **valid bare identifier** — a bare or
/// qualified column reference, or a `CAST` of one — case 2 of the
/// alias-synthesis rule in `docs/specs/multi_backend.md` §"Output-schema
/// type conformance". Every dialect agrees on this name, so an item meeting
/// this predicate needs no synthesized alias.
fn expr_yields_bare_identifier(expr: &smelt_parser::ast::Expr) -> bool {
    if expr.as_column_ref().is_some() {
        return true;
    }
    if let Some(cast) = expr.as_cast() {
        if let Some(inner) = cast.expression() {
            return expr_yields_bare_identifier(&inner);
        }
    }
    false
}

/// Splice ` AS _smelt_col{n}` into every top-level select item of `sql` that
/// falls under case 3 of the alias-synthesis rule in
/// `docs/specs/multi_backend.md` §"Output-schema type conformance": no
/// explicit alias, and an expression that is not a bare/qualified column
/// reference (or a `CAST` of one). `n` is the item's 1-based position, so
/// the synthesized name is deterministic and independent of source
/// formatting. An item with an explicit alias, a wildcard, or a struct-spread
/// call is left untouched.
///
/// Operates by text-range splice directly over `sql`, which must be
/// **source** smelt SQL — never dialect-printed output. Splicing into
/// printed SQL would reintroduce the re-parse-printed-output hazard this
/// projection redesign removes; see
/// `docs/plans/20260819-source-derived-projection.md`. Once spliced, the
/// alias is a real, bound name — every downstream consumer (the projection
/// derivation, the dialect printer, the executing backend) sees and emits it
/// the same way any user-written alias would.
fn synthesize_projection_aliases(sql: &str) -> String {
    let parse = smelt_parser::parse(sql);
    let Some(file) = File::cast(parse.syntax()) else {
        return sql.to_string();
    };
    let Some(select_stmt) = file.select_stmt() else {
        return sql.to_string();
    };
    let Some(select_list) = select_stmt.select_list() else {
        return sql.to_string();
    };

    let mut insertions: Vec<(usize, String)> = Vec::new();
    for (i, item) in select_list.items().enumerate() {
        if item.alias().is_some() {
            continue;
        }
        if item.is_wildcard() || item.is_struct_spread_call() {
            continue;
        }
        let Some(expr) = item.expression() else {
            continue;
        };
        if expr_yields_bare_identifier(&expr) {
            continue;
        }
        // Splice immediately after the *expression's* own text range, not
        // the select item's — the item node's range can extend across
        // trailing whitespace up to the next token (e.g. the `FROM`
        // keyword) when the item has no alias, which would otherwise glue
        // the synthesized alias directly onto that keyword.
        let end: usize = expr.text_range().end().into();
        insertions.push((end, format!(" AS _smelt_col{}", i + 1)));
    }

    if insertions.is_empty() {
        return sql.to_string();
    }

    // Splice from the end backward so earlier insertion offsets stay valid.
    insertions.sort_by_key(|(pos, _)| std::cmp::Reverse(*pos));
    let mut out = sql.to_string();
    for (pos, text) in insertions {
        out.insert_str(pos, &text);
    }
    out
}

/// Derive a model's [`Projection`] from its **pre-print** source `SelectStmt`.
///
/// Reads the same notion of "output column name" the analyzer's `model_schema`
/// query reads — `SelectItem::column_name`, which resolves an alias when there
/// is one and the bare column name otherwise — so the build path and the editor
/// agree on what a model's columns are (`architecture.md` §"Diagnostic parity
/// rule"). A wildcard makes the set unknowable here (its expansion needs the
/// upstream schemas the analyzer resolves lazily), so the whole projection is
/// reported as unresolvable rather than as a partial list a consumer might
/// trust.
fn derive_projection(select_stmt: &SelectStmt, ctx: &TypeContext) -> Projection {
    let Some(select_list) = select_stmt.select_list() else {
        return Projection { columns: None };
    };
    let items: Vec<_> = select_list.items().collect();
    // A bare `*`, OR a `smelt.<path>(args).*` struct-spread call: both
    // expand to however many columns the underlying schema/struct return
    // type has, a count this source-level select list cannot state on its
    // own (the struct-spread call's field count is only known once its
    // signature is resolved, and — unlike a plain aliased/bare column
    // item — the printer fans it out into N columns, not one). Treating
    // either as making the whole projection unresolvable (rather than
    // just skipping that one item) keeps every downstream item's position
    // aligned with what the printer will actually emit; see
    // `docs/specs/multi_backend.md` §"Whole-row MERGE" for why "unknown"
    // is the correct, fail-closed reading of an empty `output_columns`.
    if items
        .iter()
        .any(|item| item.is_wildcard() || item.is_struct_spread_call())
    {
        return Projection { columns: None };
    }
    let column_types = infer_select_column_types(select_stmt, ctx);
    let columns = items
        .into_iter()
        .zip(column_types)
        .map(|(item, typed)| ProjectionColumn {
            name: item.column_name(),
            typed,
        })
        .collect();
    Projection {
        columns: Some(columns),
    }
}

/// Derive a model's output column names from its already-derived [`Projection`].
///
/// Empty means *unknown* (`CompiledModel::output_columns`'s documented
/// contract): a wildcard select item, or any item lacking a resolvable name,
/// makes the whole list untrustworthy rather than partially trustworthy.
fn output_column_names(projection: &Projection) -> Vec<String> {
    let Some(columns) = &projection.columns else {
        return Vec::new();
    };
    let mut names = Vec::with_capacity(columns.len());
    for col in columns {
        match &col.name {
            Some(name) => names.push(name.clone()),
            None => return Vec::new(),
        }
    }
    names
}

fn dialect_for_backend(backend_type: BackendType) -> (SqlDialect, BackendCapabilities) {
    match backend_type {
        BackendType::DuckDB => (SqlDialect::DuckDB, BackendCapabilities::duckdb()),
        BackendType::Spark => (SqlDialect::SparkSQL, BackendCapabilities::spark()),
        BackendType::BigQuery => (SqlDialect::BigQuery, BackendCapabilities::bigquery()),
    }
}

/// Resolve `smelt.<path>` references in arbitrary SQL text by replacing
/// them with qualified table names. Legacy `smelt.ref()` / `smelt.source()` call-forms
/// are now parse errors; only the unified `smelt.<path>` form is handled here.
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

/// Whether `model` is **self-referential** — it reads its own prior output
/// via `smelt.<self>` (`docs/specs/incremental_shapes.md` §"Window independence
/// and self-referential models"). The single shared predicate for every
/// runtime consumer (the first-run bootstrap dispatch in `execute.rs`, both
/// windowed and unwindowed arms, and the output-schema fixpoint in
/// `UpstreamSchemas::from_database`), so the sites can never drift apart.
///
/// A ref is a self-edge **iff its full dotted path equals the model's
/// canonical dot-path** (`ModelFile::canonical_path`, the same key
/// `DependencyGraph::build` keys models by and suppresses self-edges with
/// (`dep == canonical`), and the same `model_name` the ordered-execution
/// proof `smelt_logical::analysis::window_independence` matches `refs`
/// against). Matching anything looser is wrong in both directions:
///
/// - **Leaf-name matching false-positives**: `smelt.gold.sessions` referenced
///   from `silver/sessions.sql` shares the leaf `sessions` but is a different
///   model.
/// - **There is no false-negative for a "relative" self-ref**, because smelt
///   has no relative-ref resolution: an unqualified `smelt.sessions_chained`
///   written inside `silver/` is refused at the diagnostic gate
///   (`UndefinedModelRef`, with a fix-it naming the canonical path) before
///   any run dispatch, and would in any case compile to a *different*
///   physical table (`<schema>.sessions_chained`, not
///   `<schema>.silver_sessions_chained`) — it is not a working self-reference
///   under any predicate. The canonical path is the only self-ref spelling
///   that resolves.
///
/// Falls back to `model.name` when `address_segments` is empty (the same
/// fallback `ModelFile::db_name_owned` uses for test-constructed models).
pub(crate) fn is_self_referential(model: &ModelFile) -> bool {
    let canonical = model.canonical_path();
    let canonical: &str = if canonical.is_empty() {
        &model.name
    } else {
        &canonical
    };
    model
        .refs
        .iter()
        .any(|r| r.smelt_ref.to_path().join(".") == canonical)
}

/// Inline `smelt.define` function-call bodies into `sql` using `fn_bodies`,
/// leaving `smelt.<path>` source references intact (`smelt_path_ref: None`).
///
/// Standalone counterpart to `SqlCompiler::expand_function_calls` for callers
/// that have a `FnBodyMap` but no compiler — notably the explain /
/// batch-safety classification path, which must derive lookback bounds from the
/// expanded SQL so a `RANGE BETWEEN INTERVAL` declared inside a function body is
/// seen. The dialect is irrelevant here: the output feeds the text-based bound
/// deriver, and a `RANGE BETWEEN INTERVAL` round-trips identically across
/// dialects. Returns `sql` unchanged when `fn_bodies` is empty.
pub fn expand_function_calls(sql: &str, fn_bodies: &FnBodyMap) -> String {
    if fn_bodies.is_empty() {
        return sql.to_string();
    }
    let bodies = Arc::new(fn_bodies.clone());
    let fn_bodies_for_fn = Arc::clone(&bodies);
    let fn_expander: SmeltFnExpander<'static> = Box::new(
        move |fn_name: &str, positional: Vec<String>, named: Vec<(String, String)>| {
            let (params, body_sql) = fn_bodies_for_fn.get(fn_name)?;
            Some(substitute_params_with_named(
                body_sql,
                params,
                &positional,
                &named,
            ))
        },
    );
    let fn_bodies_for_call = Arc::clone(&bodies);
    let path_call_expander: SmeltPathCallExpander<'static> = Box::new(
        move |segs: &[String], positional: Vec<String>, named: Vec<(String, String)>| {
            let fn_name = segs.last()?;
            let (params, body_sql) = fn_bodies_for_call.get(fn_name)?;
            Some(substitute_params_with_named(
                body_sql,
                params,
                &positional,
                &named,
            ))
        },
    );
    let parse = smelt_parser::parse(sql);
    let ctx = PrintContext {
        dialect: &SqlDialect::DuckDB,
        capabilities: &BackendCapabilities::duckdb(),
        schema: "",
        ephemeral_models: std::collections::HashSet::new(),
        cross_engine_refs: std::collections::HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: Some(fn_expander),
        smelt_path_ref: None,
        smelt_path_call: Some(path_call_expander),
    };
    smelt_dialect::print(&parse.syntax(), &ctx)
}

/// If `segs`/`positional` form a `smelt.config.var('name')` call whose name is
/// present in `vars`, return the variable's value rendered as a single-quoted
/// SQL `Text` literal. Mirrors `meta_eval::expand_config_vars` for calls the
/// printer discovers during function-body re-expansion (BUG-067) — the
/// model-level meta pass has already run by then, so an inlined body's
/// `config.var` is only visible here. A name absent from `vars` returns `None`
/// (verbatim fallback; the analyzer gates `ConfigVarNotFound`).
fn config_var_literal(
    vars: &std::collections::BTreeMap<String, String>,
    segs: &[String],
    positional: &[String],
) -> Option<String> {
    if segs.len() != 2
        || !segs[0].eq_ignore_ascii_case("config")
        || !segs[1].eq_ignore_ascii_case("var")
    {
        return None;
    }
    let arg = positional.first()?.trim();
    let name = arg
        .strip_prefix('\'')?
        .strip_suffix('\'')?
        .replace("''", "'");
    let value = vars.get(&name)?;
    Some(format!("'{}'", value.replace('\'', "''")))
}

pub struct SqlCompiler {
    config: Config,
    dialect: SqlDialect,
    capabilities: BackendCapabilities,
    /// The name of the active target this compiler was instantiated for (e.g.
    /// `"dev"`, `"prod"`). Used to resolve per-target `name:` overrides in
    /// source YAMLs (`SourceNameOverride::PerTarget`).  Empty string when no
    /// target name is set (falls back to the literal-or-default mapping).
    target_name: String,
    /// Cross-engine refs: model_name -> parquet read expression.
    /// Set externally before compilation when cross-engine references exist.
    cross_engine_refs: HashMap<String, String>,
    /// Upstream model and seed schemas, used by `apply_type_casts` to build a
    /// populated `TypeContext` so aggregate widening rules apply correctly to
    /// `smelt.<path>` ref and source columns.
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
    /// Names of models classified as state-bearing (`AggregatorColumn.state`
    /// carries a decomposed state shape on at least one column,
    /// `docs/specs/incremental_shapes.md` §"Decomposed state (rung 2) in
    /// keyed models"). Consulted by `presentation_projection` so a
    /// downstream `SELECT *` over one of these models never surfaces its
    /// `__part` state columns. Empty until rows 5-6 of
    /// `docs/outcomes/20260809-rung2-state-shapes/outcome.md` widen
    /// admission — `AggregatorColumn.state` is `None` everywhere today.
    state_bearing_models: std::collections::BTreeSet<String>,
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
    /// Compile-time variables (`smelt.yml` `vars:`), each already coerced to its
    /// `Text` rendering via the analyzer's `coerce_yaml_scalar_to_text` (so the
    /// build path and the analyzer agree, per `architecture.md` §"Diagnostic
    /// parity rule"). Consumed at compile time by the meta-language
    /// `smelt.config.var(name)` evaluator; never reaches the database engine.
    pub vars: std::collections::BTreeMap<String, String>,
    /// Workspace model / source listings (with merged tags), built from the
    /// analyzer's own `models_all` / `sources_all` Salsa queries so the build
    /// and the editor resolve `smelt.models.*` / `smelt.sources.*` identically
    /// (`architecture.md` §"Diagnostic parity rule"). Consumed at compile time
    /// by the build-path wide-reflection evaluator; never reaches the engine.
    pub model_refs: Vec<crate::meta_eval::EntityRefMeta>,
    pub source_refs: Vec<crate::meta_eval::EntityRefMeta>,
    /// Registered loader files (raw text + existence) keyed by their
    /// workspace-relative path, built from the analyzer's own `LoaderFileInput`
    /// registry so the build resolves `smelt.config.load_yaml` / `load_json`
    /// against the same content the editor validated (`architecture.md`
    /// §"Diagnostic parity rule"). Consumed at compile time by the build-path
    /// loader evaluator; never reaches the engine.
    pub loaders: std::collections::BTreeMap<String, crate::meta_eval::LoaderFileMeta>,
    /// Self-referential models whose output-schema fixpoint
    /// (`from_database`'s self-ref refinement pass) exhausted its round cap
    /// without converging. Their `models` entry is the last (unconverged)
    /// iterate — usable as a best-effort hint for type-cast wrapping, but
    /// NOT reliable enough to author DDL from: the first-run bootstrap
    /// (`execute.rs`) refuses such a model with a diagnostic instead of
    /// creating a table from an unconverged schema
    /// (`architecture.md` §"Fail-loud discipline").
    pub unconverged_self_ref_models: std::collections::HashSet<String>,
}

/// Adapts per-entity `SourceInfo`s into the LEGACY aggregate `SourcesConfig`
/// shape so `build_type_context` bakes their columns in from the start,
/// rather than via the separate `add_source_info_to_type_context` call that
/// runs too late for a FROM-clause derived table reading a per-entity
/// source directly (see the doc comment at this function's one call site,
/// `UpstreamSchemas::from_database`'s self-referential-model fixpoint).
/// Only the table name (the address's last segment) and column list matter
/// for the lookup this feeds — `build_type_context`'s `add_source_column`
/// always also registers the unqualified `{table}.{column}` key, which is
/// the one a bare `t.col` reference through an alias actually resolves
/// through — so the synthetic source/schema name is a fixed placeholder.
fn sources_config_from_source_infos(infos: &[smelt_core::SourceInfo]) -> SourcesConfig {
    let sources = infos
        .iter()
        .filter_map(|info| {
            let table_name = info.address_segments.last()?.clone();
            let columns = info
                .columns
                .iter()
                .map(|c| smelt_core::SourceColumnDef {
                    name: c.name.clone(),
                    data_type: Some(c.data_type.clone()),
                    description: None,
                    data_latency: None,
                })
                .collect();
            Some(smelt_core::SourceDef {
                name: "sources".to_string(),
                database: None,
                schema: None,
                description: None,
                tables: vec![smelt_core::SourceTableDef {
                    name: table_name,
                    identifier: None,
                    description: None,
                    columns,
                }],
            })
        })
        .collect();
    SourcesConfig { sources }
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
        models: &[ModelFile],
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
                        data_type: DataType::unknown_dynamic(),
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

        // Self-referential models: refine `model_schemas`'s own self-entry by
        // a bounded local fixpoint (`docs/specs/incremental_shapes.md` §"First-run
        // and backfill" — "First-run bootstrap for a self-referential
        // model"). `resolved_model_schema` (Salsa) cannot help here: its
        // `cycle_initial` BREAKS a genuine self-referential cycle with an
        // empty schema rather than iterating it to a fixpoint, so a self-
        // dependent column's type comes back `Unknown` (or a poisoned
        // default like `SUM`'s BIGINT fallback) from the pass above. But the
        // model's self-read exposes EXACTLY its own output columns by
        // construction — a genuine fixed point over `model_schemas[model.
        // name]` itself — and the pure functions already imported here
        // (`build_type_context`/`infer_select_column_types`) can converge on
        // it directly: re-infer the model's own top-level SELECT's column
        // types using `model_schemas` (including this model's own,
        // currently-rough self-entry) as the ref-schema source, feed the
        // result back as this model's entry, and repeat. Most shapes
        // converge in one or two rounds (e.g. `COALESCE(self.balance, 0) +
        // SUM(amount)`'s only self-dependency is on a column whose type
        // becomes concrete once fed back, not on iterating the aggregate
        // itself); the round cap below is a safety bound, not a tuned
        // parameter.
        //
        // `build_type_context` only bakes the LEGACY aggregate `sources:`
        // block into the context it builds; per-entity source columns are
        // layered in afterwards by a separate `add_source_info_to_type_
        // context` call. That later call is too late for a FROM-clause
        // derived table that itself reads a per-entity source (a query
        // shaped `SELECT … FROM (SELECT … FROM smelt.sources.x t …) inner`)
        // — `build_type_context`'s own recursive derived-table handling
        // infers the inner SELECT's column types eagerly, before the
        // per-entity call ever runs, so a straight per-entity source column
        // read through such a wrapper resolves `Unknown` regardless of any
        // self-reference. Feeding the per-entity columns in as a synthetic
        // legacy `SourcesConfig` instead sidesteps this ordering gap for the
        // fixpoint below (`sources_config_from_source_infos`) — this is a
        // local, scoped workaround for the self-referential-model schema
        // fixpoint, not a general fix to `build_type_context`'s ordering
        // (that lives in `smelt-db`, outside this phase's file scope).
        let per_entity_as_legacy_sources = sources_config_from_source_infos(&per_entity_sources);
        let mut unconverged_self_ref_models: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for model in models {
            // Shared self-edge predicate (`is_self_referential`, canonical
            // dot-path equality — the same test the first-run bootstrap
            // dispatch in `execute.rs` uses). `model_schemas` itself stays
            // keyed on `model.name` (the bare leaf, the SAME identifier
            // `StaticRefSchemaProvider::resolved_columns` looks up by);
            // only the self-edge *detection* uses the canonical path.
            if !is_self_referential(model) {
                continue;
            }
            let Some(current) = model_schemas.get(&model.name).cloned() else {
                continue;
            };
            let clean_sql = smelt_parser::strip_frontmatter(&model.content);
            let parse = smelt_parser::parse(&clean_sql);
            let Some(file) = File::cast(parse.syntax()) else {
                continue;
            };
            let Some(select_stmt) = file.select_stmt() else {
                continue;
            };

            let mut names: Vec<String> = current.iter().map(|(n, _)| n.clone()).collect();
            let mut previous_types: Vec<TypedColumn> =
                current.iter().map(|(_, t)| t.clone()).collect();

            // Fail-loud (`architecture.md` §"Fail-loud discipline"): a
            // fixpoint that exhausts its round cap without converging must
            // not silently pass off the last iterate as "the schema" —
            // record the model so the bootstrap consumer (`execute.rs`)
            // refuses with a diagnostic instead of executing DDL derived
            // from an unconverged result. Non-convergence is not expected
            // to be reachable — each round's types move monotonically
            // through the finite promotion lattice
            // (`type_comparison`/`promote_types`), so re-running the same
            // pure inference over a fed-back result reaches a fixed point
            // well inside the cap — but "not expected" is exactly what a
            // guard is for; the cap is a safety bound, not a proof.
            let mut converged = false;
            const MAX_ROUNDS: usize = 20;
            for _ in 0..MAX_ROUNDS {
                let provider = StaticRefSchemaProvider {
                    models: &model_schemas,
                    seeds: &seed_schemas,
                };
                let mut ctx = build_type_context(&file, &per_entity_as_legacy_sources, &provider);
                add_source_info_to_type_context(&per_entity_sources, &mut ctx);
                let next_types = infer_select_column_types(&select_stmt, &ctx);

                if next_types == previous_types {
                    converged = true;
                    break;
                }
                previous_types = next_types.clone();
                if names.len() != next_types.len() {
                    // A struct-spread or other name/type-count mismatch this
                    // simple positional fixpoint doesn't model — keep
                    // whatever names we already have and stop refining
                    // rather than zip mismatched vectors.
                    names.truncate(next_types.len());
                }
                let refined: Vec<(String, TypedColumn)> = names
                    .iter()
                    .cloned()
                    .zip(next_types.iter().cloned())
                    .collect();
                model_schemas.insert(model.name.clone(), refined);
            }
            if !converged {
                tracing::warn!(
                    "self-referential model '{}': output-schema fixpoint did not converge \
                     within {MAX_ROUNDS} rounds; its resolved schema is unreliable and a \
                     first-run bootstrap will be refused",
                    model.name
                );
                unconverged_self_ref_models.insert(model.name.clone());
            }
        }

        // Compile-time `vars:` for `smelt.config.var`. Parse the `vars:` block
        // and coerce each scalar to its `Text` rendering using the SAME helpers
        // the analyzer uses (`smelt-db::config_vars`), so a value the editor
        // resolves and a value the build splices are byte-identical.
        let vars = std::fs::read_to_string(project_dir.join("smelt.yml"))
            .ok()
            .and_then(|text| smelt_db::config_vars::parse_vars_from_yaml(&text))
            .map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            smelt_db::config_vars::coerce_yaml_scalar_to_text(v, k).0,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Workspace model / source listings for wide reflection
        // (`smelt.models.*` / `smelt.sources.*`). Reuse the analyzer's own
        // `models_all` / `sources_all` queries so the build resolves the same
        // listing, tag merge, path normalisation, and sort order the editor
        // sees (per `architecture.md` §"Diagnostic parity rule"). Models are
        // workspace-wide; sources are per-project, so union over every project.
        let model_refs: Vec<crate::meta_eval::EntityRefMeta> = smelt_db::models_all(db, workspace)
            .iter()
            .map(|m| crate::meta_eval::EntityRefMeta {
                path: m.path.clone(),
                name: m.name.clone(),
                tags: m.tags.clone(),
            })
            .collect();
        let mut source_refs: Vec<crate::meta_eval::EntityRefMeta> = workspace
            .projects(db)
            .iter()
            .copied()
            .flat_map(|project| {
                smelt_db::sources_all(db, project)
                    .iter()
                    .map(|s| crate::meta_eval::EntityRefMeta {
                        path: s.path.clone(),
                        name: s.name.clone(),
                        tags: s.tags.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        // Each project's `sources_all` is path-sorted internally; re-sort the
        // cross-project union so the workspace-wide listing is globally
        // path-sorted (`meta_language.md` §"smelt.sources accessors": sorted
        // ascending by `path`). `models_all` is already workspace-wide sorted.
        source_refs.sort_by(|a, b| a.path.cmp(&b.path));

        // Registered loader files for `smelt.config.load_yaml` / `load_json`.
        // Reuse the analyzer's own `LoaderFileInput` registry (populated by
        // `init_db` via `register_loader_files_from_disk`) so the build reads the
        // same loader content the editor validated (per `architecture.md`
        // §"Diagnostic parity rule").
        let loaders: std::collections::BTreeMap<String, crate::meta_eval::LoaderFileMeta> =
            workspace
                .loader_files(db)
                .iter()
                .map(|input| {
                    (
                        input.path(db).to_string(),
                        crate::meta_eval::LoaderFileMeta {
                            text: input.text(db).to_string(),
                            exists: input.exists(db),
                        },
                    )
                })
                .collect();

        Ok(Self {
            models: model_schemas,
            seeds: seed_schemas,
            sources,
            per_entity_sources,
            vars,
            model_refs,
            source_refs,
            loaders,
            unconverged_self_ref_models,
        })
    }
}

impl SqlCompiler {
    pub(crate) fn new(config: Config, target: &Target) -> Self {
        let (dialect, capabilities) = match target.backend_type() {
            Ok(bt) => dialect_for_backend(bt),
            Err(_) => (SqlDialect::DuckDB, BackendCapabilities::duckdb()),
        };
        Self {
            config,
            dialect,
            capabilities,
            target_name: String::new(),
            cross_engine_refs: HashMap::new(),
            upstream_schemas: Arc::new(UpstreamSchemas::default()),
            fn_bodies: None,
            state_bearing_models: std::collections::BTreeSet::new(),
        }
    }

    /// Set the active target name so per-target `name:` overrides in source
    /// YAMLs resolve correctly.  Call this immediately after `new` when the
    /// target name is known (e.g. from `CompilerRegistry::new`).
    pub(crate) fn set_target_name(&mut self, name: &str) {
        self.target_name = name.to_string();
    }

    /// Set cross-engine ref mappings (model_name -> parquet read expression).
    pub(crate) fn set_cross_engine_refs(&mut self, refs: HashMap<String, String>) {
        self.cross_engine_refs = refs;
    }

    /// Provide upstream model/seed/source schemas so `apply_type_casts` can
    /// resolve `smelt.<path>` ref and source column types correctly.
    pub(crate) fn set_upstream_schemas(&mut self, schemas: Arc<UpstreamSchemas>) {
        self.upstream_schemas = schemas;
    }

    /// Provide the set of model names classified as state-bearing, so a
    /// downstream `SELECT *` reading one of them gets its `__part` state
    /// columns hidden by `presentation_projection`
    /// (`docs/specs/incremental_shapes.md` §"Decomposed state (rung 2) in
    /// keyed models" → "Presentation projection").
    pub(crate) fn set_state_bearing_models(&mut self, models: std::collections::BTreeSet<String>) {
        self.state_bearing_models = models;
    }

    /// The presented-column map `presentation_projection` needs: keyed by
    /// `state_bearing_models`, valued from the *public* schema
    /// (`upstream_schemas.models`) — no new source of truth for "which
    /// columns are presented". A state-bearing model absent from
    /// `upstream_schemas.models` (its schema wasn't resolvable) is simply
    /// omitted; `presentation_projection` then treats any relation to it as
    /// non-state-bearing rather than refusing, since this compiler has no
    /// column list to hide behind anyway.
    fn presentation_map(&self) -> std::collections::BTreeMap<String, Vec<String>> {
        self.state_bearing_models
            .iter()
            .filter_map(|name| {
                self.upstream_schemas
                    .models
                    .get(name)
                    .map(|cols| (name.clone(), cols.iter().map(|(n, _)| n.clone()).collect()))
            })
            .collect()
    }

    /// Hide decomposed state columns behind a presentation projection
    /// before this SQL reaches ref-name rewriting/printing, so a refusal
    /// can still name the user's `smelt.models.*` path
    /// (`docs/specs/incremental_shapes.md` §"Decomposed state (rung 2) in
    /// keyed models" → "Presentation projection"). A no-op (returns `sql`
    /// unchanged) when no state-bearing model is in scope.
    fn hide_state_columns(&self, sql: &str) -> Result<String> {
        let state_bearing = self.presentation_map();
        smelt_logical::maintenance::emit::presentation_projection(sql, &state_bearing)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Provide pre-resolved `smelt.fn.*` function bodies for SQL emission.
    ///
    /// Maps leaf function name → (param_names, body_sql). When set, `smelt.fn.*`
    /// calls in compiled models are expanded inline. When not set (the default),
    /// they pass through verbatim.
    ///
    /// For multi-target setups use [`CompilerRegistry::set_function_bodies_all`].
    #[allow(dead_code)]
    pub(crate) fn set_function_bodies(&mut self, bodies: FnBodyMap) {
        self.fn_bodies = Some(Arc::new(bodies));
    }

    /// Like [`SqlCompiler::set_function_bodies`] but takes an already-shared
    /// `Arc<FnBodyMap>` so callers can register the same map on multiple
    /// compilers without cloning the underlying allocation.
    fn set_function_bodies_arc(&mut self, bodies: Arc<FnBodyMap>) {
        self.fn_bodies = Some(bodies);
    }

    /// Build the build-path meta-evaluation context: compile-time `vars:` for
    /// `smelt.config.var`, the upstream model/seed column lists for
    /// `smelt.columns_of` reflection, the project's `smelt.define` bodies
    /// (so a `columns_of` inside a function body is reachable), and the
    /// workspace model/source listings for `smelt.models.*` / `smelt.sources.*`
    /// wide reflection. Borrows the compiler's `UpstreamSchemas` / `fn_bodies`.
    fn meta_ctx(&self) -> crate::meta_eval::MetaEvalContext<'_> {
        crate::meta_eval::MetaEvalContext {
            vars: &self.upstream_schemas.vars,
            columns: self.reflection_columns(),
            fn_bodies: self.fn_bodies.as_deref(),
            models: &self.upstream_schemas.model_refs,
            sources: &self.upstream_schemas.source_refs,
            loaders: &self.upstream_schemas.loaders,
        }
    }

    /// Build the `smelt.columns_of` reflection map (entity leaf name → column
    /// list) from the upstream model and seed schemas. Each column is reduced to
    /// the `ColumnRef` fields the build-path evaluator splices (`name`,
    /// `is_numeric` via the analyzer's `DataType::is_numeric`, and the `type`
    /// display string). Models take precedence over seeds on a name clash.
    fn reflection_columns(
        &self,
    ) -> std::collections::BTreeMap<String, Vec<crate::meta_eval::ColumnRefMeta>> {
        let to_metas = |cols: &Vec<(String, TypedColumn)>| {
            cols.iter()
                .map(|(name, tc)| crate::meta_eval::ColumnRefMeta {
                    name: name.clone(),
                    type_name: tc.data_type.to_string(),
                    is_numeric: tc.data_type.is_numeric(),
                    is_decimal: tc.data_type.is_decimal(),
                    is_string: tc.data_type.is_string(),
                    // meta_language.md §ColumnRef: temporal family = Date/Timestamp/Time
                    // (excludes Interval, unlike DataType::is_temporal() which is for the
                    // type-constraint Temporal set and includes Interval).
                    is_temporal: matches!(
                        tc.data_type,
                        DataType::Date | DataType::Time | DataType::Timestamp { .. }
                    ),
                    is_integer: tc.data_type.is_integer(),
                    is_boolean: tc.data_type.is_boolean(),
                })
                .collect::<Vec<_>>()
        };
        let mut out = std::collections::BTreeMap::new();
        for (name, cols) in &self.upstream_schemas.seeds {
            out.insert(name.clone(), to_metas(cols));
        }
        for (name, cols) in &self.upstream_schemas.models {
            out.insert(name.clone(), to_metas(cols));
        }
        out
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
            let mut ctx = build_type_context(&file, &self.upstream_schemas.sources, &provider);
            // Per-entity source columns, same as `apply_type_casts`.
            add_source_info_to_type_context(&self.upstream_schemas.per_entity_sources, &mut ctx);
            Some(ctx)
        } else {
            None
        };

        let dialect_name = match self.dialect {
            SqlDialect::DuckDB => "duckdb",
            SqlDialect::SparkSQL => "spark",
            SqlDialect::PostgreSQL => "postgres",
            SqlDialect::BigQuery => "bigquery",
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
        //
        // The expander also resolves `smelt.config.var('x')` (BUG-067): the
        // model-level meta pass runs before parsing, so a `config.var` inside
        // a `smelt.define` body is only ever seen here, when the printer
        // re-expands the inlined body.
        let vars = self.upstream_schemas.vars.clone();
        let path_call_expander: Option<SmeltPathCallExpander<'static>> =
            Some(match self.fn_bodies.as_ref() {
                Some(bodies) => {
                    let bodies = Arc::clone(bodies);
                    let expander: SmeltPathCallExpander<'static> = Box::new(
                        move |segs: &[String],
                              positional: Vec<String>,
                              named: Vec<(String, String)>| {
                            if let Some(literal) = config_var_literal(&vars, segs, &positional) {
                                return Some(literal);
                            }
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
                        move |segs: &[String],
                              positional: Vec<String>,
                              _named: Vec<(String, String)>| {
                            config_var_literal(&vars, segs, &positional)
                        },
                    );
                    expander
                }
            });

        (as_struct_emitter, fn_expander, path_call_expander)
    }

    /// Build a `SmeltPathRefResolver` for a specific `schema` string.
    ///
    /// Per architecture.md §"Default materialization name mapping":
    /// - All persisted paths → `{schema}.{segs.join("_")}`
    /// - `smelt.sources.<path>` with `name:` override → the override verbatim.
    /// - `smelt.sources.<path>` without override → same unified default mapping.
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
        let target_name = self.target_name.clone();
        let cross_engine_refs = self.cross_engine_refs.clone();
        let per_entity_sources = self.upstream_schemas.per_entity_sources.clone();
        let ephemerals = ephemeral_names.clone();

        Box::new(move |segs: &[String]| {
            match segs {
                // smelt.sources.<path>: per-entity source with a `name:` override
                // emits that override verbatim (literal) or the target-specific
                // entry (per-target map). Without any override, sources follow
                // the same unified default mapping as models and functions:
                //   `<target_schema>.<segs.join("_")>`
                // (architecture.md §"Default materialization name mapping").
                // For `smelt.sources.raw.users` with schema `main` this is
                // `main.sources_raw_users`. Use `name: raw.users` in the source
                // YAML when the external table lives at a different location.
                segs if !segs.is_empty() && segs[0] == "sources" => {
                    // Per-entity source with an explicit `name:` override wins;
                    // resolve against the active target name for per-target maps.
                    if let Some(src_info) = per_entity_sources
                        .iter()
                        .find(|s| s.address_segments.as_slice() == segs)
                    {
                        if src_info.name_override.is_some() {
                            return Some(src_info.db_name_for_target(&target_name, &schema));
                        }
                    }
                    // No override (or not found in per-entity registry) — unified
                    // default mapping: `<schema>.<segs.join("_")>`.
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

    /// Hard-error when a model is `refresh: materialized_view` and the resolved
    /// backend's capabilities have no native incremental-view maintenance.
    ///
    /// No backend advertises `supports_native_ivm` today, so this always fires
    /// for `refresh: materialized_view` models — never a silent fallback to
    /// `refresh: incremental` or a full-refresh table
    /// (`docs/specs/materialized_view.md` §"No silent fallback"). Called from
    /// every compile entry point (`compile`, `compile_with_sql`,
    /// `compile_with_sql_and_ephemerals`) so the gate applies uniformly
    /// whether or not ephemeral inlining or SQL transformation happened first.
    fn check_native_ivm_gate(&self, model: &ModelFile) -> Result<()> {
        let refresh = self
            .config
            .get_refresh_with_metadata(&model.name, model.metadata.as_ref().map(|b| b.as_ref()));
        if refresh == RefreshStrategy::MaterializedView && !self.capabilities.supports_native_ivm {
            anyhow::bail!(
                "`refresh: materialized_view` requires native incremental-view maintenance; \
                 this engine has none — use `refresh: incremental` with `grain: key` for \
                 smelt-driven maintenance."
            );
        }
        Ok(())
    }

    /// Compile a model's SQL by replacing smelt.ref() calls with table references
    pub fn compile(&self, model: &ModelFile, schema: &str) -> Result<CompiledModel> {
        self.check_native_ivm_gate(model)?;

        // Strip frontmatter to avoid parse errors from YAML metadata
        let clean_content = smelt_parser::strip_frontmatter(&model.content);
        // Evaluate in-model meta-language constructs (e.g. list spread) to plain
        // SQL before codegen so the analyzer-accepted surface never reaches the
        // engine (build-path half of the diagnostic-parity rule).
        let clean_content =
            crate::meta_eval::expand_in_model_meta(&clean_content, &self.meta_ctx());
        // Hide decomposed state columns behind a presentation projection
        // while `smelt.models.*` ref names are still recoverable in the
        // text (before printing rewrites them to physical table names) —
        // see `hide_state_columns`.
        let clean_content = self.hide_state_columns(&clean_content)?;
        // Bind a real `_smelt_col{n}` alias into the source text for every
        // select item case 3 of the alias-synthesis rule covers, before this
        // parse — see `synthesize_projection_aliases` and
        // `docs/specs/multi_backend.md` §"Output-schema type conformance".
        let clean_content = synthesize_projection_aliases(&clean_content);
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
        // The projection — output column names and their inferred types — is
        // derived once, here, from the pre-print source CST (`parse`), before
        // dialect lowering. Every consumer (the type-cast wrap, the output
        // column list) reads this single derivation; neither re-parses the
        // dialect-printed SQL below. See `docs/specs/multi_backend.md`
        // §"Output-schema type conformance".
        let projection = self.derive_projection_for(&parse.syntax());

        let compiled_sql = smelt_dialect::print(&parse.syntax(), &ctx);

        // Type-conforming cast insertion: wrap SELECT columns with CASTs so
        // backend output types match smelt's type inference exactly.
        let compiled_sql = self.apply_type_casts(&compiled_sql, &projection);

        // Get materialization: SQL metadata > smelt.yml > default
        let materialization = self.config.get_materialization_with_metadata(
            &model.name,
            model.metadata.as_ref().map(|b| b.as_ref()),
        );

        Ok(CompiledModel {
            // Use the full address-based DB name (e.g. "staging_stg_events")
            // so the backend creates/accesses the correct table.
            name: model.db_name_owned(),
            output_columns: output_column_names(&projection),
            sql: compiled_sql,
            materialization,
        })
    }

    /// Build the populated `TypeContext` a [`Projection`] derivation needs,
    /// from upstream model/seed/source schemas, for the given (pre-print)
    /// parsed `file`.
    ///
    /// Without this populated context, every ref column resolves to Unknown
    /// and SUM falls through to BIGINT — silently corrupting financial
    /// aggregates. See bug #3 in
    /// `docs/research/20260417-0.3-regression-triage.md`.
    fn build_projection_type_context(&self, file: &File) -> TypeContext {
        let provider = StaticRefSchemaProvider {
            models: &self.upstream_schemas.models,
            seeds: &self.upstream_schemas.seeds,
        };
        let mut ctx = build_type_context(file, &self.upstream_schemas.sources, &provider);
        // Per-entity `sources/*.yml` columns (Phase 6) — the same two-path
        // source resolution `smelt-db`'s `type_context()` query performs.
        // Without this, every per-entity source column resolves to Unknown
        // and `SUM(val)` falls through to the concrete `BigInt` default,
        // so the `_smelt_typed` wrapper truncates fractional aggregates
        // (design fork F4's runtime-side sibling; end-to-end pinned by the
        // property loop's G-01 cell with fractional values).
        add_source_info_to_type_context(&self.upstream_schemas.per_entity_sources, &mut ctx);
        ctx
    }

    /// Derive this compiler's [`Projection`] from a parsed, **pre-print**
    /// source AST — the single call every compile entry point uses so the
    /// projection is always read off the model's own select list, never off
    /// dialect-lowered output. `syntax` must be the root of a
    /// `smelt_parser::parse` call over source smelt SQL (never over already
    /// dialect-printed SQL). Returns an unresolvable (`columns: None`)
    /// projection when the syntax doesn't cast to a `File`/`SelectStmt` at
    /// all (e.g. a non-SELECT top level), mirroring `derive_projection`'s
    /// own wildcard/non-SELECT fail-closed behaviour.
    fn derive_projection_for(&self, syntax: &smelt_parser::syntax_kind::SyntaxNode) -> Projection {
        File::cast(syntax.clone())
            .and_then(|file| {
                let select_stmt = file.select_stmt()?;
                let type_ctx = self.build_projection_type_context(&file);
                Some(derive_projection(&select_stmt, &type_ctx))
            })
            .unwrap_or(Projection { columns: None })
    }

    /// Wrap SELECT columns with CASTs based on the already-derived [`Projection`].
    ///
    /// Returns `sql` unchanged if the projection has no statically-known
    /// columns (e.g. a wildcard) or no concrete types (e.g. models
    /// referencing other models via smelt.ref() where upstream schemas
    /// aren't yet available).
    fn apply_type_casts(&self, sql: &str, projection: &Projection) -> String {
        let Some(columns) = &projection.columns else {
            return sql.to_string();
        };

        // Only apply casts if we have concrete types for at least one column
        let has_concrete = columns
            .iter()
            .any(|c| !matches!(c.typed.data_type, DataType::Unknown(_) | DataType::Null));
        if !has_concrete {
            return sql.to_string();
        }

        // Every statically-resolved item now carries a real name: an explicit
        // alias, its own bare/qualified column name, or (for anything else)
        // the `_smelt_col{n}` alias `synthesize_projection_aliases` already
        // spliced into the source SQL before this projection was derived —
        // see `docs/specs/multi_backend.md` §"Output-schema type
        // conformance". The `_smelt_col{n}` fallback below is therefore
        // defensive only: it can be reached only if an item's expression is
        // itself unparseable, in which case the name matches what the
        // splice would have bound anyway.
        let col_names: Vec<String> = columns
            .iter()
            .enumerate()
            .map(|(i, col)| {
                col.name
                    .clone()
                    .unwrap_or_else(|| format!("_smelt_col{}", i + 1))
            })
            .collect();
        let col_name_refs: Vec<&str> = col_names.iter().map(|s| s.as_str()).collect();
        let col_type_refs: Vec<DataType> =
            columns.iter().map(|c| c.typed.data_type.clone()).collect();

        wrap_with_type_casts(sql, &col_name_refs, &col_type_refs, self.dialect)
    }

    /// Compile a model with custom SQL (e.g., for transformed queries).
    ///
    /// Restricted to crate-internal use: external callers must use
    /// [`SqlCompiler::compile_with_sql_and_ephemerals`] so ephemeral inlining is
    /// never accidentally bypassed.
    #[allow(dead_code)]
    pub(crate) fn compile_with_sql(
        &self,
        model: &ModelFile,
        schema: &str,
        sql: &str,
    ) -> Result<CompiledModel> {
        self.check_native_ivm_gate(model)?;

        let sql = crate::meta_eval::expand_in_model_meta(sql, &self.meta_ctx());
        let sql = self.hide_state_columns(&sql)?;
        // Bind a real `_smelt_col{n}` alias into the source text for every
        // select item case 3 of the alias-synthesis rule covers, before this
        // parse — see `synthesize_projection_aliases` and
        // `docs/specs/multi_backend.md` §"Output-schema type conformance".
        let sql = synthesize_projection_aliases(&sql);
        let parse = smelt_parser::parse(&sql);
        // The projection is derived here, from `parse` — the pre-print
        // source AST — never from `compiled_sql` below. See
        // `derive_projection_for` and `docs/specs/multi_backend.md`
        // §"Output-schema type conformance".
        let projection = self.derive_projection_for(&parse.syntax());
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
            output_columns: output_column_names(&projection),
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
        EphemeralResolver::new(
            ephemeral_models,
            &self.dialect,
            &self.capabilities,
            schema,
            &self.upstream_schemas.vars,
        )
    }

    /// Like `compile_with_sql`, but also inlines referenced ephemeral models as CTEs.
    pub fn compile_with_sql_and_ephemerals(
        &self,
        model: &ModelFile,
        schema: &str,
        sql: &str,
        resolver: &EphemeralResolver,
    ) -> Result<CompiledModel> {
        self.check_native_ivm_gate(model)?;

        let ephemeral_refs: HashSet<&str> = resolver
            .ephemeral_names
            .iter()
            .map(|s| s.as_str())
            .collect();

        let sql = crate::meta_eval::expand_in_model_meta(sql, &self.meta_ctx());
        let sql = self.hide_state_columns(&sql)?;
        // Bind a real `_smelt_col{n}` alias into the source text for every
        // select item case 3 of the alias-synthesis rule covers, before this
        // parse — see `synthesize_projection_aliases` and
        // `docs/specs/multi_backend.md` §"Output-schema type conformance".
        let sql = synthesize_projection_aliases(&sql);
        let parse = smelt_parser::parse(&sql);
        // The projection is derived here, from `parse` — the pre-print
        // source AST, before both dialect lowering AND the ephemeral-CTE
        // prepend below. `prepend_ephemeral_ctes` only adds a `WITH`
        // preamble in front of the existing top-level SELECT; it never
        // touches that SELECT's own item list, so this projection is still
        // exactly right for `final_sql`'s output columns. See
        // `derive_projection_for` and `docs/specs/multi_backend.md`
        // §"Output-schema type conformance".
        let projection = self.derive_projection_for(&parse.syntax());
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
        let compiled_sql = self.apply_type_casts(&compiled_sql, &projection);

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
            // `prepend_ephemeral_ctes` still operates on printed SQL — that
            // stays deferred by design (`docs/plans/20260819-source-derived-
            // projection.md` Scope → Explicitly deferred: `WITH` spelling is
            // dialect-invariant, so this textual merge is a style problem,
            // not a projection-correctness one). Nothing downstream of it
            // re-parses the result: `output_columns` below reads `projection`,
            // derived above from source, not from `final_sql`.
            prepend_ephemeral_ctes(&compiled_sql, &cte_list)
        };

        let materialization = self.config.get_materialization_with_metadata(
            &model.name,
            model.metadata.as_ref().map(|b| b.as_ref()),
        );

        Ok(CompiledModel {
            // Use the full address-based DB name (e.g. "staging_stg_events").
            name: model.db_name_owned(),
            output_columns: output_column_names(&projection),
            sql: final_sql,
            materialization,
        })
    }

    /// Inline `smelt.define` function-call bodies into `sql` while leaving
    /// `smelt.<path>` source references intact (NOT rewritten to physical
    /// names). The lookback-bound deriver matches sources by their
    /// `smelt.<path>` name and scans for `RANGE BETWEEN INTERVAL` / window
    /// patterns; feeding it this expanded form makes a bound (or a bare,
    /// non-derivable window) declared *inside* a function body visible, instead
    /// of silently defaulting to a zero-lookback read. Returns `sql` unchanged
    /// when no function bodies are registered (the common case for models that
    /// call no `smelt.define` functions).
    pub fn expand_function_calls(&self, sql: &str) -> String {
        match self.fn_bodies.as_ref() {
            Some(bodies) => expand_function_calls(sql, bodies),
            None => sql.to_string(),
        }
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
        vars: &std::collections::BTreeMap<String, String>,
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
                vars,
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
        vars: &std::collections::BTreeMap<String, String>,
    ) -> Vec<(String, String)> {
        let ephemeral_refs: HashSet<&str> = ephemeral_names.iter().map(|s| s.as_str()).collect();
        let clean_sql = smelt_parser::strip_frontmatter(raw_sql);
        let clean_sql = crate::meta_eval::expand_in_model_meta(
            &clean_sql,
            &crate::meta_eval::MetaEvalContext::vars_only(vars),
        );
        let parse = smelt_parser::parse(&clean_sql);

        // Build a path-ref resolver that maps smelt.<path> to either
        // __smelt_<segs.join("_")> (if ephemeral) or schema.<segs.join("_")>.
        // Sources follow the same unified default mapping as all other paths
        // (architecture.md §"Default materialization name mapping"). The `name:`
        // override is not available here (this resolver is used for a lighter
        // compile path that does not load per-entity source info); the full
        // override-aware resolver lives in `make_path_ref_resolver_with_ephemerals`.
        let ephemerals_owned: HashSet<String> = ephemeral_names.clone();
        let schema_owned = schema.to_string();
        let path_ref_resolver: SmeltPathRefResolver<'static> =
            Box::new(move |segs: &[String]| match segs {
                // All non-empty paths → either ephemeral CTE or schema.<segs.join("_")>.
                // This includes sources: `smelt.sources.raw.users` → `main.sources_raw_users`.
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
pub(crate) fn prepend_ephemeral_ctes(sql: &str, cte_list: &[(String, String)]) -> String {
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
        self.check_native_ivm_gate(model)?;

        let clean_content = smelt_parser::strip_frontmatter(&model.content);
        let clean_content =
            crate::meta_eval::expand_in_model_meta(&clean_content, &self.meta_ctx());
        // Bind a real `_smelt_col{n}` alias into the source text for every
        // select item case 3 of the alias-synthesis rule covers, before this
        // parse — see `synthesize_projection_aliases` and
        // `docs/specs/multi_backend.md` §"Output-schema type conformance".
        let clean_content = synthesize_projection_aliases(&clean_content);
        let parse = smelt_parser::parse(&clean_content);
        // The projection is derived here, from `parse` — the pre-print
        // source AST, before both dialect lowering AND the ephemeral-CTE
        // prepend below; see the identical note in
        // `compile_with_sql_and_ephemerals`.
        let projection = self.derive_projection_for(&parse.syntax());

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
        let compiled_sql = self.apply_type_casts(&compiled_sql, &projection);

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
                } else {
                    None
                }
            })
            .collect();

        // Prepend ephemeral CTEs if any. This still operates on printed SQL
        // — deferred by design, see the identical note in
        // `compile_with_sql_and_ephemerals`. `output_columns` below reads
        // `projection`, derived above from source, not from `final_sql`.
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
            output_columns: output_column_names(&projection),
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
            let mut compiler = SqlCompiler::new(config.clone(), target);
            compiler.set_target_name(name);
            compilers.insert(name.clone(), compiler);
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
    /// `smelt.<path>` ref or source provides — it doesn't care which
    /// backend ultimately materialises the upstream model.
    pub fn set_upstream_schemas_all(&mut self, schemas: Arc<UpstreamSchemas>) {
        for compiler in self.compilers.values_mut() {
            compiler.set_upstream_schemas(schemas.clone());
        }
    }

    /// Set the state-bearing model set on every compiler in the registry —
    /// resolved once per project (`build_state_bearing_models` in
    /// `execute.rs`) and shared across targets, since which models carry
    /// decomposed state doesn't depend on which backend ultimately
    /// materialises the consumer.
    pub fn set_state_bearing_models_all(&mut self, models: std::collections::BTreeSet<String>) {
        for compiler in self.compilers.values_mut() {
            compiler.set_state_bearing_models(models.clone());
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
    use smelt_core::config::{ModelConfig, Target};
    use smelt_core::RefInfo;
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
            settings: None,
            project: None,
            dataset: None,
            location: None,
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
                settings: None,
                project: None,
                dataset: None,
                location: None,
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
            target: None,
            state: Default::default(),
            maintenance: None,
            probes: Default::default(),
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler = SqlCompiler::new(config, &make_test_target());

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(compiled.sql.contains("FROM main.raw_events"));
        assert!(!compiled.sql.contains("smelt.raw_events"));
    }

    /// A compiler carrying a state-bearing model map compiles a downstream
    /// `SELECT *` into the presented column list instead of a bare `*`
    /// (`docs/specs/incremental_shapes.md` §"Decomposed state (rung 2) in
    /// keyed models" → "Presentation projection").
    #[test]
    fn compile_hides_state_columns_from_downstream_star() {
        let sql = "SELECT * FROM smelt.models.agg";
        let model = ModelFile {
            name: "downstream".to_string(),
            path: "models/downstream.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let mut compiler = SqlCompiler::new(config, &make_test_target());

        let mut models = HashMap::new();
        models.insert(
            "agg".to_string(),
            vec![
                (
                    "customer_id".to_string(),
                    TypedColumn::new(DataType::BigInt, false),
                ),
                (
                    "avg_amount".to_string(),
                    TypedColumn::new(DataType::Double, true),
                ),
            ],
        );
        compiler.set_upstream_schemas(Arc::new(UpstreamSchemas {
            models,
            ..Default::default()
        }));
        let mut state_bearing = std::collections::BTreeSet::new();
        state_bearing.insert("agg".to_string());
        compiler.set_state_bearing_models(state_bearing);

        let compiled = compiler.compile(&model, "main").unwrap();

        assert!(
            compiled.sql.contains("customer_id") && compiled.sql.contains("avg_amount"),
            "expected the presented column list, got: {}",
            compiled.sql
        );
        assert!(
            !compiled.sql.contains('*'),
            "the bare wildcard must not survive compilation: {}",
            compiled.sql
        );
    }

    /// An empty state-bearing model set compiles the identical SQL a
    /// pre-presentation-projection compile would have produced — the
    /// parity guard for every project that never touches decomposed state.
    #[test]
    fn compile_is_unchanged_without_state_bearing_models() {
        let sql = "SELECT * FROM smelt.models.agg";
        let model = ModelFile {
            name: "downstream".to_string(),
            path: "models/downstream.sql".into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        let compiler_baseline = SqlCompiler::new(config.clone(), &make_test_target());
        let compiler_empty_state = SqlCompiler::new(config, &make_test_target());

        let baseline = compiler_baseline.compile(&model, "main").unwrap();
        let empty_state = compiler_empty_state.compile(&model, "main").unwrap();

        assert_eq!(baseline.sql, empty_state.sql);
        assert!(baseline.sql.contains('*'));
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
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

    /// `refresh: materialized_view` on DuckDB (no native IVM) is a hard error —
    /// never a silent fallback to `keyed` or a full-refresh table
    /// (`docs/specs/materialized_view.md` §"No silent fallback").
    #[test]
    fn test_materialized_view_hard_errors_without_native_ivm() {
        let model = ModelFile {
            name: "mv_model".to_string(),
            path: "models/mv_model.sql".into(),
            content: "SELECT device_id, COUNT(*) AS n FROM smelt.raw_events GROUP BY device_id"
                .to_string(),
            refs: vec![],
            parse_errors: Vec::new(),
            metadata: Some(Box::new(smelt_core::metadata::ModelMetadata {
                materialization: Some(Materialization::Table),
                refresh: Some(RefreshStrategy::MaterializedView),
                ..Default::default()
            })),
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("mv_model.sql".into()),
            address_segments: Vec::new(),
        };

        let config = make_test_config();
        // make_test_target() is `duckdb`, which sets `supports_native_ivm = false`.
        let compiler = SqlCompiler::new(config, &make_test_target());

        let err = compiler
            .compile(&model, "main")
            .expect_err("refresh: materialized_view on DuckDB must hard-error");
        let message = err.to_string();
        assert!(
            message.contains("requires native incremental-view maintenance"),
            "expected the §\"No silent fallback\" hard error, got: {}",
            message
        );
        assert!(
            message.contains("use `refresh: incremental` with `grain: key`"),
            "expected the hard error to point at `refresh: incremental` + `grain: key`, got: {}",
            message
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
            address_segments: Vec::new(),
        };

        let mut config = make_test_config();
        config.models.insert(
            "test_model".to_string(),
            ModelConfig {
                materialization: Some(Materialization::Table),
                timeseries: None,
                refresh: None,
                grain: None,
                unique_key: None,
                safety_overrides: None,
                batched_retired: (),
                merge_key: None,
                tags: Vec::new(),
                target: None,
                format: None,
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
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
            &std::collections::BTreeMap::new(),
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
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
            &std::collections::BTreeMap::new(),
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
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
            &std::collections::BTreeMap::new(),
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
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
            &std::collections::BTreeMap::new(),
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

    /// The `compile_with_sql` entry point (crate-internal, so unit-tested
    /// here rather than from `tests/`) must derive `output_columns` from the
    /// model's source select list, never from dialect-printed SQL: Spark's
    /// `supports_qualify = false` lowers `QUALIFY` to an outer `SELECT *
    /// FROM (<original>) _q WHERE <predicate>` — a wildcard select item.
    /// `output_columns` must still resolve from the model's own, perfectly
    /// well-named source projection (`id, rn`), not from that printed
    /// wildcard form — the same regression the ephemeral-aware entry points
    /// guard against in `tests/projection_source_derived.rs`.
    #[test]
    fn compile_with_sql_qualify_lowering_does_not_erase_output_columns() {
        let sql = "SELECT id, ROW_NUMBER() OVER (PARTITION BY id ORDER BY ts DESC) AS rn \
                   FROM t QUALIFY rn = 1";
        let model = ModelFile {
            name: "qualify_model".to_string(),
            path: "models/qualify_model.sql".into(),
            content: sql.to_string(),
            refs: vec![],
            parse_errors: Vec::new(),
            metadata: None,
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("qualify_model.sql".into()),
            address_segments: Vec::new(),
        };

        let target = Target {
            target_type: "spark".to_string(),
            database: None,
            schema: "default".to_string(),
            connect_url: Some("sc://localhost".to_string()),
            catalog: None,
            warehouse: None,
            format: None,
            settings: None,
            project: None,
            dataset: None,
            location: None,
        };
        let compiled = SqlCompiler::new(make_test_config(), &target)
            .compile_with_sql(&model, "main", sql)
            .unwrap();

        assert_eq!(
            compiled.output_columns,
            vec!["id".to_string(), "rn".to_string()],
            "the QUALIFY-to-subquery lowering's outer `SELECT *` must not make \
             a statically-known source projection look unknown: sql = {}",
            compiled.sql
        );
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
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

    /// The type-cast wrapper now names its columns and infers their types
    /// from the model's **source** `SelectStmt`, before dialect lowering —
    /// never by re-parsing the printed SQL. Before this fix, BigQuery's
    /// `MEDIAN` lowering (`ARRAY_AGG(x IGNORE NULLS ORDER BY x)` inside a
    /// `CASE`) did not parse back as smelt SQL, so the alias was lost and the
    /// wrapper emitted a positional `_col2` the warehouse rejected
    /// (`Unrecognized name: _col2`, seen live).
    #[test]
    fn bigquery_median_lowering_keeps_its_alias_through_the_cast_wrapper() {
        // The `CAST` gives the wrapper a concrete type to engage on.
        let sql = "SELECT CAST(d AS DATE) AS d, MEDIAN(val) AS med_val \
                   FROM raw.events GROUP BY d";

        let model = ModelFile {
            name: "holistic".to_string(),
            path: "models/holistic.sql".into(),
            content: sql.to_string(),
            refs: vec![],
            parse_errors: Vec::new(),
            metadata: None,
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("holistic.sql".into()),
            address_segments: Vec::new(),
        };

        let target = Target {
            target_type: "bigquery".to_string(),
            database: None,
            schema: "main".to_string(),
            connect_url: None,
            catalog: None,
            warehouse: None,
            format: None,
            settings: None,
            project: Some("p".to_string()),
            dataset: Some("main".to_string()),
            location: Some("US".to_string()),
        };
        let compiled = SqlCompiler::new(make_test_config(), &target)
            .compile(&model, "main")
            .unwrap();

        assert!(
            !compiled.sql.contains("_col"),
            "no positional fallback name may reach the backend: {}",
            compiled.sql
        );
        assert!(
            compiled.sql.contains("med_val"),
            "the model's own alias must survive: {}",
            compiled.sql
        );
        assert!(
            !compiled.sql.contains("MEDIAN("),
            "the BigQuery lowering must still apply: {}",
            compiled.sql
        );
        // Regression (measured live 2026-08-19, `gate_bigquery::
        // append_only_partition_pool_upholds_equivalence_on_bigquery`,
        // `recipe_holistic_agg`): the type-cast wrapper used to re-parse the
        // ALREADY dialect-lowered SQL (this MEDIAN call became an
        // `ARRAY_AGG`/`CASE`/`CAST(... AS FLOAT64)` expression by the time
        // `apply_type_casts` ran). `FLOAT64` is a GoogleSQL spelling
        // `smelt_types::parse_type` doesn't recognise, so the median
        // expression's own type went unresolved — and an unsound fallback in
        // `type_inference::binary::promote_numeric_operands_for_op` let the
        // division's unresolved operand silently adopt the OTHER, known
        // operand's type (the literal `2`'s `SmallInt`), emitting
        // `CAST(med_val AS SMALLINT)` around an exact-median value and
        // rounding interpolated medians (e.g. `-284.5` -> `-285`) before they
        // ever left BigQuery.
        //
        // Now that the cast wrap's types come from the **source** CST
        // (`MEDIAN(val)`, never the lowered `ARRAY_AGG`/`CASE` form), the
        // registry resolves `MEDIAN`'s return type as `Double` regardless of
        // its operand — so a `CAST(med_val AS FLOAT64)` (BigQuery's spelling
        // of `Double`) is the correct, exact-width output. What must never
        // reappear is a narrowing integer/fixed-point cast.
        assert!(
            !compiled.sql.contains("CAST(med_val AS SMALLINT")
                && !compiled.sql.contains("CAST(med_val AS INT")
                && !compiled.sql.contains("CAST(med_val AS NUMERIC")
                && !compiled.sql.contains("CAST(med_val AS BIGNUMERIC"),
            "an unresolved median type must not be cast to a guessed narrower \
             integer/fixed-point type, silently rounding interpolated medians: {}",
            compiled.sql
        );
        assert!(
            compiled.sql.contains("CAST(med_val AS FLOAT64)"),
            "MEDIAN's registry-declared Double return type should still \
             surface as an exact-width FLOAT64 cast on BigQuery: {}",
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
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
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("final_users.sql".into()),
            // TODO Phase 5: compute address_segments from model path so canonical_path() is correct.
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
            &std::collections::BTreeMap::new(),
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

    /// BUG-009: when the arg is a qualified name (contains `.`) and the param
    /// appears as a FROM-table reference in the body, the fix must alias the arg
    /// at the FROM position rather than text-replacing all occurrences.
    ///
    /// Without the fix: `SELECT main.orders.*, … FROM main.orders` — DuckDB
    /// rejects `main.orders.*` as "syntax error at or near *".
    /// With the fix:   `SELECT source.*, … FROM main.orders AS source` — valid.
    #[test]
    fn substitute_tableexpr_qualified_arg_uses_from_alias() {
        let body = "(SELECT source.*, revenue - cost AS margin FROM source)";
        let params = params_no_defaults(&["source"]);
        let positional = vec!["main.orders".to_string()];

        let result = substitute_params_with_named(body, &params, &positional, &[]);

        // FROM clause must alias the qualified arg to the param name.
        assert!(
            result.contains("FROM main.orders AS source"),
            "expected `FROM main.orders AS source` in result, got: {result}"
        );
        // source.* must stay verbatim so it resolves through the alias.
        assert!(
            result.contains("source.*"),
            "expected `source.*` to remain unchanged (resolves through alias), got: {result}"
        );
        // The qualified name must NOT appear in a star wildcard position.
        assert!(
            !result.contains("main.orders.*"),
            "must not produce `main.orders.*` (invalid DuckDB syntax), got: {result}"
        );
    }

    /// BUG-009 regression: a CTE-name arg (no dot) must still use the
    /// original text-replacement path — aliasing should NOT be applied.
    #[test]
    fn substitute_tableexpr_cte_arg_uses_text_replacement() {
        let body = "(SELECT source.*, revenue - cost AS margin FROM source)";
        let params = params_no_defaults(&["source"]);
        let positional = vec!["my_cte".to_string()]; // no dot — CTE name

        let result = substitute_params_with_named(body, &params, &positional, &[]);

        // Normal replacement: source → my_cte everywhere.
        assert!(
            result.contains("my_cte.*"),
            "expected `my_cte.*` (normal replacement for CTE arg), got: {result}"
        );
        assert!(
            result.contains("FROM my_cte"),
            "expected `FROM my_cte` (normal replacement), got: {result}"
        );
        assert!(
            !result.contains("source"),
            "param name must be fully replaced for CTE arg, got: {result}"
        );
    }

    /// Builds a `ModelFile` with an explicit address (layer path) and refs
    /// extracted from real SQL, for `is_self_referential` predicate tests.
    fn make_addressed_model(name: &str, address_segments: &[&str], sql: &str) -> ModelFile {
        ModelFile {
            name: name.to_string(),
            path: format!("models/{}.sql", address_segments.join("/")).into(),
            content: sql.to_string(),
            refs: extract_refs_from_sql(sql),
            parse_errors: Vec::new(),
            metadata: None,
            kind: smelt_core::ModelKind::Sql,
            model_id: smelt_core::ModelId::from_path("test.sql".into()),
            address_segments: address_segments.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// The shared self-edge predicate matches a layered model's fully
    /// qualified (canonical dot-path) self-reference — the only self-ref
    /// spelling that resolves (see `is_self_referential`'s doc comment).
    #[test]
    fn self_ref_predicate_matches_canonical_layered_self_ref() {
        let model = make_addressed_model(
            "sessions",
            &["silver", "sessions"],
            "SELECT a FROM smelt.silver.sessions",
        );
        assert!(is_self_referential(&model));
    }

    /// A cross-layer reference that merely SHARES the model's leaf name
    /// (`gold.sessions` referenced from `silver/sessions.sql`) is NOT a
    /// self-edge. This pins the false positive the previous leaf-name
    /// matching had: under `to_path().last() == model.name` this model
    /// would (wrongly) classify as self-referential.
    #[test]
    fn self_ref_predicate_rejects_same_leaf_model_in_another_layer() {
        let model = make_addressed_model(
            "sessions",
            &["silver", "sessions"],
            "SELECT a FROM smelt.gold.sessions",
        );
        assert!(!is_self_referential(&model));
    }

    /// An unlayered model's self-reference is its bare name — canonical
    /// path and leaf coincide, so the predicate matches (the g_08 fixture
    /// shape: `models/running_balance.sql` reading `smelt.running_balance`).
    #[test]
    fn self_ref_predicate_matches_unlayered_self_ref() {
        let model = make_addressed_model(
            "running_balance",
            &["running_balance"],
            "SELECT a FROM smelt.running_balance",
        );
        assert!(is_self_referential(&model));
    }

    /// Test-constructed models with empty `address_segments` fall back to
    /// `model.name` (the same fallback `db_name_owned` uses), so the
    /// predicate still works for fixtures that never ran discovery.
    #[test]
    fn self_ref_predicate_falls_back_to_name_when_segments_empty() {
        let mut model = make_addressed_model(
            "running_balance",
            &["running_balance"],
            "SELECT a FROM smelt.running_balance",
        );
        model.address_segments = Vec::new();
        assert!(is_self_referential(&model));
    }

    /// A model with no self-edge at all (ordinary upstream ref) is not
    /// self-referential.
    #[test]
    fn self_ref_predicate_rejects_plain_upstream_ref() {
        let model = make_addressed_model(
            "daily",
            &["gold", "daily"],
            "SELECT a FROM smelt.silver.events",
        );
        assert!(!is_self_referential(&model));
    }
}
