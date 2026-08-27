//! Dialect-aware CST printer.
//!
//! Walks a Rowan CST and emits SQL text, performing dialect-specific rewrites:
//! - `smelt.<path>` → resolved table name (via `SmeltPathRef`/`SmeltPathCall`)
//! - QUALIFY → subquery rewrite (when `!caps.supports_qualify`)
//! - ARRAY[1,2,3] → ARRAY(1,2,3) (when `!caps.supports_array_literal`)
//! - DATE '2024-01-01' → DATE('2024-01-01') (when `!caps.supports_date_literal`)
//! - expr::type → CAST(expr AS type) (when `!caps.supports_double_colon_cast`)
//!
//! The default behavior is verbatim: tokens (including whitespace and comments)
//! are emitted exactly as they appear in the source. This guarantees an identity
//! property for DuckDB with no refs/sources.

use smelt_parser::ast::{BinaryExpr, SmeltAsStructCall, SmeltPathCall, SmeltPathRef};
use smelt_parser::syntax_kind::{SyntaxElement, SyntaxKind, SyntaxNode};
use smelt_parser::{CastExpr, FunctionCall};

use std::collections::{HashMap, HashSet};

use crate::position::classify as classify_position;
use crate::{BackendCapabilities, SqlDialect};
use smelt_types::signatures::Position;
use smelt_types::{BuiltinRegistry, Emission, RewriteId};

/// Emitter closure type for `smelt.as_struct(alias [EXCEPT cols])`.
/// Called with `(alias, except_columns)` → emitted SQL, or `None` to pass through.
pub type AsStructEmitter<'a> = Box<dyn Fn(&str, &[String]) -> Option<String> + 'a>;

/// Expander closure type for `smelt.fn.*` calls.
/// Called with `(fn_name, positional_arg_sqls, named_args)` → expanded SQL, or `None`.
pub type SmeltFnExpander<'a> =
    Box<dyn Fn(&str, Vec<String>, Vec<(String, String)>) -> Option<String> + 'a>;

/// Resolver closure type for `smelt.<path>` value references (`SMELT_PATH_REF`).
/// Called with path segments (everything after `smelt`); returns backend SQL or `None` to
/// emit verbatim.
pub type SmeltPathRefResolver<'a> = Box<dyn Fn(&[String]) -> Option<String> + 'a>;

/// Expander closure type for `smelt.<path>(<args>)` call forms (`SMELT_PATH_CALL`).
/// Called with `(path_segments, positional_arg_sqls, named_arg_sqls)` → expanded SQL, or `None`
/// to emit verbatim.
pub type SmeltPathCallExpander<'a> =
    Box<dyn Fn(&[String], Vec<String>, Vec<(String, String)>) -> Option<String> + 'a>;

/// Context for dialect-aware printing.
pub struct PrintContext<'a> {
    pub dialect: &'a SqlDialect,
    pub capabilities: &'a BackendCapabilities,
    pub schema: &'a str,
    /// Model names that are ephemeral — refs to these emit `__smelt_{name}` instead of `schema.name`.
    pub ephemeral_models: HashSet<&'a str>,
    /// Cross-engine refs: model_name -> `read_parquet('{path}/**/*.parquet', hive_partitioning=true)`.
    /// When a ref is in this map, the parquet expression is emitted instead of `schema.model`.
    pub cross_engine_refs: HashMap<String, String>,
    /// Emitter for `smelt.as_struct(alias [EXCEPT cols])`.
    ///
    /// Called with `(alias, except_columns)` and returns the backend-specific SQL string.
    /// `None` = pass through verbatim (backward compat for tests / contexts without schema info).
    pub smelt_as_struct: Option<AsStructEmitter<'a>>,
    /// Expander for `smelt.fn.*` calls.
    ///
    /// Called with `(fn_name, positional_arg_sqls, named_args)` and returns the expanded SQL.
    /// `None` = pass through verbatim (backward compat for tests / contexts without function info).
    pub smelt_fn: Option<SmeltFnExpander<'a>>,
    /// Resolver for `smelt.<path>` value references (`SMELT_PATH_REF`).
    ///
    /// Called with path segments (everything after `smelt`) and returns the backend SQL string.
    /// `None` = pass through verbatim (backward compat for callers that have not configured
    /// path resolution).
    pub smelt_path_ref: Option<SmeltPathRefResolver<'a>>,
    /// Expander for `smelt.<path>(<args>)` call forms (`SMELT_PATH_CALL`).
    ///
    /// Called with `(path_segments, positional_arg_sqls, named_arg_sqls)` and returns the
    /// expanded SQL. `None` = pass through verbatim.
    pub smelt_path_call: Option<SmeltPathCallExpander<'a>>,
}

/// Print a CST node as dialect-specific SQL.
pub fn print(node: &SyntaxNode, ctx: &PrintContext) -> String {
    let mut out = String::new();
    print_node(node, ctx, &mut out);
    out
}

fn print_node(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    match node.kind() {
        SyntaxKind::SMELT_AS_STRUCT_CALL => {
            if let Some(ref emitter) = ctx.smelt_as_struct {
                if let Some(call) = SmeltAsStructCall::cast(node.clone()) {
                    if let Some(alias) = call.alias() {
                        let except = call.except_columns();
                        if let Some(sql) = emitter(&alias, &except) {
                            out.push_str(&sql);
                            return;
                        }
                    }
                }
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::SMELT_PATH_REF => {
            if let Some(path_ref) = SmeltPathRef::cast(node.clone()) {
                let segs = path_ref.segments();

                // Try explicit resolver first.
                let resolved = ctx
                    .smelt_path_ref
                    .as_ref()
                    .and_then(|resolver| resolver(&segs));

                // Fall back to built-in resolution based on the path namespace:
                //   smelt.models.<name>   → schema.name  (or cross-engine / ephemeral)
                //   smelt.sources.<src>.<tbl>  → src.tbl
                let resolved = resolved.or_else(|| match segs.as_slice() {
                    [ns, name] if ns == "models" => {
                        if ctx.ephemeral_models.contains(name.as_str()) {
                            Some(format!("__smelt_{}", name))
                        } else if let Some(parquet_expr) = ctx.cross_engine_refs.get(name.as_str())
                        {
                            Some(parquet_expr.clone())
                        } else {
                            Some(format!("{}.{}", ctx.schema, name))
                        }
                    }
                    [ns, src, tbl] if ns == "sources" => Some(format!("{}.{}", src, tbl)),
                    _ => None,
                });

                if let Some(sql) = resolved {
                    out.push_str(&sql);
                    // Re-emit trailing trivia (whitespace/comments) captured
                    // inside this node by the parser's look-ahead skip_trivia
                    // call. These are direct-child tokens after the SMELT_PATH
                    // sub-node (e.g. the space before an `AS` alias).
                    for child in node.children_with_tokens() {
                        if let SyntaxElement::Token(t) = child {
                            if t.kind().is_trivia() {
                                out.push_str(t.text());
                            }
                        }
                    }
                    return;
                }
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::SMELT_PATH_CALL => {
            if let (Some(ref expander), Some(path_call)) =
                (&ctx.smelt_path_call, SmeltPathCall::cast(node.clone()))
            {
                let segs = path_call.segments();
                let positional: Vec<String> = path_call
                    .arg_list()
                    .map(|al| {
                        al.positional_args()
                            .into_iter()
                            .map(|arg| {
                                let mut s = String::new();
                                print_node(arg.syntax(), ctx, &mut s);
                                s
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let mut named: Vec<(String, String)> = path_call
                    .arg_list()
                    .map(|al| {
                        al.named_params()
                            .filter_map(|np| {
                                let name = np.name()?;
                                let expr = np.value_expr()?;
                                let mut s = String::new();
                                print_node(expr.syntax(), ctx, &mut s);
                                Some((name, s))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                // Merge block `PASSING <name> AS (<body>)` fragment bindings into
                // the named-argument set. A PASSING clause binds parameter
                // `<name>` to `<body>` exactly as a `<name> => <body>` inline
                // named argument would — the type-checker rejects supplying the
                // same parameter both ways, so there is no collision to resolve.
                // The body is printed through `ctx` so any nested smelt
                // constructs inside the fragment are expanded too.
                for clause in path_call.passing_clauses() {
                    if let Some(name) = clause.name() {
                        let body = if let Some(expr) = clause.body_expr() {
                            let mut s = String::new();
                            print_node(expr.syntax(), ctx, &mut s);
                            s
                        } else if let Some(text) = clause.body_text() {
                            text
                        } else {
                            continue;
                        };
                        named.push((name, body));
                    }
                }
                if let Some(expanded) = expander(&segs, positional, named) {
                    // Detect FROM-position: the SMELT_PATH_CALL node's parent
                    // must be TABLE_REF, and TABLE_REF's parent must be
                    // FROM_CLAUSE. FROM-position only — JOIN/LATERAL positions
                    // are not handled here.
                    let in_from_position = node
                        .parent()
                        .filter(|p| p.kind() == SyntaxKind::TABLE_REF)
                        .and_then(|table_ref| table_ref.parent())
                        .map(|gp| gp.kind() == SyntaxKind::FROM_CLAUSE)
                        .unwrap_or(false);

                    // Detect user-supplied alias: scan TABLE_REF children that
                    // come after this SMELT_PATH_CALL node.  An alias is present
                    // when there is an AS_KW or a bare IDENT following the call.
                    let has_explicit_alias = in_from_position && {
                        node.parent()
                            .filter(|p| p.kind() == SyntaxKind::TABLE_REF)
                            .map(|table_ref| smelt_path_call_has_explicit_alias(&table_ref, node))
                            .unwrap_or(false)
                    };

                    if in_from_position && !has_explicit_alias {
                        // Synthesise `(<expanded>) AS __smelt_t<start_offset>`.
                        // Using the source byte offset of the call makes the
                        // alias stable per call site with no shared mutable
                        // state — valid across all models in a project because
                        // each model is printed independently.
                        let offset = u32::from(node.text_range().start());
                        let body = reexpand_call_body(&expanded, ctx);
                        let body_trimmed = body.trim_end();
                        out.push('(');
                        out.push_str(body_trimmed);
                        out.push_str(&format!(") AS __smelt_t{offset}"));
                    } else {
                        out.push_str(&reexpand_call_body(&expanded, ctx));
                    }
                    return;
                }
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::SMELT_PATH_CALL_STAR => {
            // `smelt.<path>(args).*` — lower to per-field projections when
            // the expander provides a brace-struct literal body; fall back to
            // verbatim otherwise.
            if let Some(expanded) = expand_smelt_path_call_star(node, ctx) {
                out.push_str(&expanded);
                return;
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::FUNCTION_CALL => {
            if let Some(fc) = FunctionCall::cast(node.clone()) {
                if let Some(name) = fc.name() {
                    if emit_registered_function(node, &fc, &name, ctx, out) {
                        return;
                    }
                }
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::BINARY_EXPR => {
            if emit_registered_operator(node, ctx, out) {
                return;
            }
            print_children(node, ctx, out);
        }
        SyntaxKind::PIPE_QUERY if !ctx.capabilities.supports_pipe_syntax => {
            print_pipe_rewrite(node, ctx, out);
        }
        SyntaxKind::SELECT_STMT if !ctx.capabilities.supports_qualify => {
            print_select_with_qualify_rewrite(node, ctx, out);
        }
        SyntaxKind::ARRAY_LITERAL if !ctx.capabilities.supports_array_literal => {
            print_array_rewrite(node, ctx, out);
        }
        SyntaxKind::SELECT_LIST | SyntaxKind::GROUP_BY_CLAUSE
            if !ctx.capabilities.supports_trailing_commas =>
        {
            print_strip_trailing_commas(node, ctx, out);
        }
        SyntaxKind::CAST_EXPR if !ctx.capabilities.supports_double_colon_cast => {
            print_cast_rewrite(node, ctx, out);
        }
        // Top-level smelt DSL declarations that are not SQL: suppress them so
        // they never reach the backend engine.  `SMELT_DEFINE` and
        // `SMELT_EXTERN` carry function bodies / extern signatures that the
        // compiler inlines via the function-expander closure above; they must
        // not be emitted verbatim.  `SMELT_RECORD_DECL` carries type
        // declarations used by the analyzer but invisible to the engine.
        SyntaxKind::SMELT_DEFINE
        | SyntaxKind::SMELT_EXTERN
        | SyntaxKind::SMELT_RECORD_DECL
        | SyntaxKind::SMELT_TEST => {
            // Emit nothing — drop the declaration from the compiled SQL.
        }
        _ => {
            print_children(node, ctx, out);
        }
    }
}

/// Lower a `PIPE_QUERY` node to standard SQL when the backend does not support
/// native pipe syntax (`supports_pipe_syntax = false`).
///
/// **Passthrough collapse (Lowering rule 1):** Contiguous passthrough stages
/// (`FROM`, pre-aggregation `WHERE`, trailing `SELECT`, `ORDER BY`, `LIMIT`,
/// `DISTINCT`) are collapsed into a single `SELECT … FROM … WHERE … ORDER BY …
/// LIMIT …`.
///
/// **Projection-editing operators (Lowering rule 4):** `EXTEND`, `SET`, `DROP`,
/// `RENAME`, and `AS` lower to a re-projection. When such a stage follows a
/// stage that already fixed the projection (i.e. we have accumulated a prior SQL
/// fragment), the prior query is wrapped as a subquery:
/// `SELECT <new_projection> FROM (<prior_query>)`.
///
/// **AGGREGATE (Lowering rule 2/3/5):** lowers to `SELECT keys, aggs … GROUP BY keys`;
/// a following `WHERE` becomes `HAVING`; multiple aggregations nest as subqueries.
///
/// **JOIN (Lowering rule 1):** the right-hand table is folded into the same FROM
/// clause as the pipe input (left side). If aggregate/order/limit state has
/// accumulated first, the current fragment is flushed to a subquery before joining.
///
/// **Set operations (Lowering rule 6):** left-fold `(q1), (q2), …` into a chain of
/// binary `UNION / INTERSECT / EXCEPT` operations.
///
/// **`SET`/`DROP`/`RENAME` on non-DuckDB backends:** these use DuckDB column-selection
/// extensions and fall back to verbatim pipe syntax on other backends.
///
/// Emitted form:
/// ```text
/// SELECT [DISTINCT] <select_list> FROM <from_body> [WHERE <pred>] [ORDER BY …] [LIMIT …]
/// ```
fn print_pipe_rewrite(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    use SyntaxKind::*;

    // ── Collect all pipe stages in order ──────────────────────────────────

    let with_clause = node.children().find(|c| c.kind() == WITH_CLAUSE);
    let from_clause = node.children().find(|c| c.kind() == FROM_CLAUSE);

    // Collect the FROM body (everything after the FROM keyword).
    let from_body = from_clause
        .as_ref()
        .map(|fc| collect_from_body(fc, ctx))
        .unwrap_or_default();

    // Collected stages (in order).
    let stages: Vec<SyntaxNode> = node.children().filter(|c| c.kind() == PIPE_STAGE).collect();

    // Check for any stages we can't handle → verbatim.
    //
    // DuckDB-only: SET, DROP, RENAME — handled on DuckDB via REPLACE/EXCLUDE/RENAME
    //   extensions; on non-DuckDB backends they are unhandled and cause verbatim.
    // Unknown op_kind (None): verbatim.
    let has_unhandled = stages.iter().any(|s| {
        let op = s.children().find_map(|c| {
            let k = c.kind();
            if matches!(
                k,
                PIPE_OP_WHERE
                    | PIPE_OP_SELECT
                    | PIPE_OP_EXTEND
                    | PIPE_OP_SET
                    | PIPE_OP_DROP
                    | PIPE_OP_RENAME
                    | PIPE_OP_AS
                    | PIPE_OP_AGGREGATE
                    | PIPE_OP_ORDER_BY
                    | PIPE_OP_LIMIT
                    | PIPE_OP_JOIN
                    | PIPE_OP_UNION
                    | PIPE_OP_INTERSECT
                    | PIPE_OP_EXCEPT
                    | PIPE_OP_DISTINCT
            ) {
                Some(k)
            } else {
                None
            }
        });
        match op {
            None => true,
            // SET/DROP/RENAME are only handled when the backend supports them; others → verbatim.
            Some(PIPE_OP_SET) | Some(PIPE_OP_DROP) | Some(PIPE_OP_RENAME) => {
                !ctx.capabilities.supports_pipe_set_drop_rename
            }
            _ => false,
        }
    });

    if has_unhandled {
        print_children(node, ctx, out);
        return;
    }

    // ── Two-pass lowering ─────────────────────────────────────────────────
    //
    // We process stages left-to-right, accumulating a "current SQL fragment".
    // The fragment starts as just the FROM source.
    //
    // Passthrough stages (WHERE, ORDER BY, LIMIT, DISTINCT) are accumulated
    // into the current fragment's clauses.
    //
    // Projection-fixing stages (SELECT) set the SELECT list and prevent
    // further WHERE from being pushed into the same level.
    //
    // Projection-editing stages (EXTEND, SET, DROP, RENAME, AS) wrap the
    // current fragment as a subquery and start a fresh outer SELECT.

    // Current accumulated fragment.
    let mut acc_where: Vec<String> = Vec::new();
    let mut acc_select: Option<String> = None;
    let mut acc_order_by: Option<String> = None;
    let mut acc_limit: Option<String> = None;
    let mut acc_distinct = false;
    // HAVING: set when a WHERE follows an AGGREGATE stage.
    let mut acc_having: Vec<String> = Vec::new();
    // GROUP BY body: retained after an AGGREGATE stage so HAVING can be emitted correctly.
    let mut acc_group_by: Option<String> = None;
    // Tracks whether the most recent data-producing stage was an AGGREGATE.
    // When true, a subsequent WHERE stage lowers to HAVING.
    let mut after_aggregate = false;
    // The "inner source" that goes in FROM. Starts as the FROM-clause table.
    let mut inner_source = from_body;

    for stage in &stages {
        let op_kind = stage.children().find_map(|c| {
            let k = c.kind();
            if matches!(
                k,
                PIPE_OP_WHERE
                    | PIPE_OP_SELECT
                    | PIPE_OP_EXTEND
                    | PIPE_OP_SET
                    | PIPE_OP_DROP
                    | PIPE_OP_RENAME
                    | PIPE_OP_AS
                    | PIPE_OP_AGGREGATE
                    | PIPE_OP_ORDER_BY
                    | PIPE_OP_LIMIT
                    | PIPE_OP_JOIN
                    | PIPE_OP_UNION
                    | PIPE_OP_INTERSECT
                    | PIPE_OP_EXCEPT
                    | PIPE_OP_DISTINCT
            ) {
                Some(k)
            } else {
                None
            }
        });

        match op_kind {
            Some(PIPE_OP_WHERE) => {
                let body = collect_where_body(stage);
                if after_aggregate {
                    // WHERE after AGGREGATE → HAVING clause.
                    acc_having.push(body);
                } else if acc_select.is_some() {
                    // WHERE after SELECT: wrap existing fragment into subquery,
                    // then add this WHERE on the outer level.
                    let prior = build_select_fragment_with_having(
                        &inner_source,
                        &acc_where,
                        &acc_select,
                        &acc_order_by,
                        &acc_limit,
                        acc_distinct,
                        &acc_group_by,
                        &acc_having,
                    );
                    inner_source = format!("({prior})");
                    acc_where = Vec::new();
                    acc_select = None;
                    acc_order_by = None;
                    acc_limit = None;
                    acc_distinct = false;
                    acc_having = Vec::new();
                    acc_group_by = None;
                    after_aggregate = false;
                    acc_where.push(body);
                } else {
                    acc_where.push(body);
                }
            }
            Some(PIPE_OP_SELECT) => {
                acc_select = Some(collect_select_body(stage));
            }
            Some(PIPE_OP_ORDER_BY) => {
                acc_order_by = Some(collect_order_by_body(stage));
            }
            Some(PIPE_OP_LIMIT) => {
                acc_limit = Some(collect_limit_body(stage));
            }
            Some(PIPE_OP_DISTINCT) => {
                acc_distinct = true;
            }
            Some(PIPE_OP_EXTEND) => {
                // Wrap current fragment as subquery, emit SELECT *, <extend_body>.
                let prior = build_select_fragment_with_having(
                    &inner_source,
                    &acc_where,
                    &acc_select,
                    &acc_order_by,
                    &acc_limit,
                    acc_distinct,
                    &acc_group_by,
                    &acc_having,
                );
                let extend_expr = collect_stage_body_text(stage, ctx, PIPE_OP_EXTEND);
                inner_source = format!("({prior})");
                acc_where = Vec::new();
                acc_select = Some(format!("*, {extend_expr}"));
                acc_order_by = None;
                acc_limit = None;
                acc_distinct = false;
                acc_having = Vec::new();
                acc_group_by = None;
                after_aggregate = false;
            }
            Some(PIPE_OP_SET) => {
                // SET replaces column values using DuckDB's REPLACE extension:
                //   SELECT * REPLACE (expr AS col, ...) FROM (prior)
                // This is only reachable when ctx.dialect == DuckDB (non-DuckDB
                // backends are rejected in the has_unhandled scan above).
                let prior = build_select_fragment_with_having(
                    &inner_source,
                    &acc_where,
                    &acc_select,
                    &acc_order_by,
                    &acc_limit,
                    acc_distinct,
                    &acc_group_by,
                    &acc_having,
                );
                let set_body = collect_stage_body_text(stage, ctx, PIPE_OP_SET);
                inner_source = format!("({prior})");
                acc_where = Vec::new();
                // Parse "col = expr, col2 = expr2" → emit "expr AS col, expr2 AS col2"
                // inside a REPLACE clause so existing columns are updated in-place.
                let assignments = parse_set_assignments(&set_body);
                let replace_list = assignments
                    .iter()
                    .map(|(col, expr)| format!("{expr} AS {col}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                acc_select = Some(format!("* REPLACE ({replace_list})"));
                acc_order_by = None;
                acc_limit = None;
                acc_distinct = false;
                acc_having = Vec::new();
                acc_group_by = None;
                after_aggregate = false;
            }
            Some(PIPE_OP_DROP) => {
                // DROP <col>, ... using DuckDB's EXCLUDE extension:
                //   SELECT * EXCLUDE (col1, col2) FROM (prior)
                // This is only reachable when ctx.dialect == DuckDB (non-DuckDB
                // backends are rejected in the has_unhandled scan above).
                let prior = build_select_fragment_with_having(
                    &inner_source,
                    &acc_where,
                    &acc_select,
                    &acc_order_by,
                    &acc_limit,
                    acc_distinct,
                    &acc_group_by,
                    &acc_having,
                );
                let drop_body = collect_stage_body_text(stage, ctx, PIPE_OP_DROP);
                inner_source = format!("({prior})");
                acc_where = Vec::new();
                let cols = parse_column_list(&drop_body);
                let exclude_list = cols.join(", ");
                acc_select = Some(format!("* EXCLUDE ({exclude_list})"));
                acc_order_by = None;
                acc_limit = None;
                acc_distinct = false;
                acc_having = Vec::new();
                acc_group_by = None;
                after_aggregate = false;
            }
            Some(PIPE_OP_RENAME) => {
                // RENAME old AS new, ... using DuckDB's RENAME extension:
                //   SELECT * RENAME (old AS new, ...) FROM (prior)
                // This is only reachable when ctx.dialect == DuckDB (non-DuckDB
                // backends are rejected in the has_unhandled scan above).
                let prior = build_select_fragment_with_having(
                    &inner_source,
                    &acc_where,
                    &acc_select,
                    &acc_order_by,
                    &acc_limit,
                    acc_distinct,
                    &acc_group_by,
                    &acc_having,
                );
                let rename_body = collect_stage_body_text(stage, ctx, PIPE_OP_RENAME);
                inner_source = format!("({prior})");
                acc_where = Vec::new();
                let pairs = parse_rename_pairs(&rename_body);
                let rename_list = pairs
                    .iter()
                    .map(|(old, new)| format!("{old} AS {new}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                acc_select = Some(format!("* RENAME ({rename_list})"));
                acc_order_by = None;
                acc_limit = None;
                acc_distinct = false;
                acc_having = Vec::new();
                acc_group_by = None;
                after_aggregate = false;
            }
            Some(PIPE_OP_AS) => {
                // AS <alias> — wrap the current fragment with an alias.
                let prior = build_select_fragment_with_having(
                    &inner_source,
                    &acc_where,
                    &acc_select,
                    &acc_order_by,
                    &acc_limit,
                    acc_distinct,
                    &acc_group_by,
                    &acc_having,
                );
                let alias = collect_as_alias(stage);
                inner_source = if let Some(a) = alias {
                    format!("({prior}) AS {a}")
                } else {
                    format!("({prior})")
                };
                acc_where = Vec::new();
                acc_select = None;
                acc_order_by = None;
                acc_limit = None;
                acc_distinct = false;
                acc_having = Vec::new();
                acc_group_by = None;
                after_aggregate = false;
            }
            Some(PIPE_OP_AGGREGATE) => {
                // AGGREGATE <agg_expr> [AS alias] [GROUP BY <keys>]
                //
                // Flush the current accumulated fragment as the inner source,
                // then set up accumulators so the fragment builder emits:
                //   SELECT <group_keys>, <agg_exprs> FROM (<prior>) GROUP BY <group_keys>
                //
                // Multiple AGGREGATE stages nest: each one wraps the previous as a subquery.
                // A subsequent |> WHERE stage sets after_aggregate=true and pushes to acc_having.
                let prior = build_select_fragment_with_having(
                    &inner_source,
                    &acc_where,
                    &acc_select,
                    &acc_order_by,
                    &acc_limit,
                    acc_distinct,
                    &acc_group_by,
                    &acc_having,
                );

                let (agg_body, group_body) = collect_aggregate_parts(stage, ctx);

                // Use the prior fragment as the subquery source.
                inner_source = format!("({prior})");

                if group_body.is_empty() {
                    // Full-table aggregation (no GROUP BY).
                    acc_select = Some(agg_body);
                    acc_group_by = None;
                } else {
                    // Grouped aggregation: keys first, then aggregates (per spec).
                    acc_select = Some(format!("{group_body}, {agg_body}"));
                    acc_group_by = Some(group_body);
                }

                // Reset all other accumulators.
                acc_where = Vec::new();
                acc_order_by = None;
                acc_limit = None;
                acc_distinct = false;
                acc_having = Vec::new();
                after_aggregate = true;
            }
            Some(PIPE_OP_JOIN) => {
                // JOIN folds into the same FROM clause when possible (Lowering rule 1).
                // If we're after an aggregate (or have ORDER BY/LIMIT pending), flush first.
                if after_aggregate || acc_order_by.is_some() || acc_limit.is_some() {
                    let prior = build_select_fragment_with_having(
                        &inner_source,
                        &acc_where,
                        &acc_select,
                        &acc_order_by,
                        &acc_limit,
                        acc_distinct,
                        &acc_group_by,
                        &acc_having,
                    );
                    let join_text = collect_join_clause_text(stage, ctx);
                    inner_source = format!("({prior}) {join_text}");
                    acc_where = Vec::new();
                    acc_select = None;
                    acc_order_by = None;
                    acc_limit = None;
                    acc_distinct = false;
                    acc_having = Vec::new();
                    acc_group_by = None;
                    after_aggregate = false;
                } else {
                    // Fold JOIN into the same FROM clause.
                    let join_text = collect_join_clause_text(stage, ctx);
                    inner_source = format!("{inner_source} {join_text}");
                }
            }
            Some(PIPE_OP_UNION) | Some(PIPE_OP_INTERSECT) | Some(PIPE_OP_EXCEPT) => {
                // Flush current fragment as the left side, then left-fold the set-op operands.
                let left = build_select_fragment_with_having(
                    &inner_source,
                    &acc_where,
                    &acc_select,
                    &acc_order_by,
                    &acc_limit,
                    acc_distinct,
                    &acc_group_by,
                    &acc_having,
                );
                let set_expr = build_set_op_expression(&left, stage, ctx);
                // Wrap as subquery so further stages can fold on top.
                inner_source = format!("({set_expr})");
                acc_where = Vec::new();
                acc_select = None;
                acc_order_by = None;
                acc_limit = None;
                acc_distinct = false;
                acc_having = Vec::new();
                acc_group_by = None;
                after_aggregate = false;
            }
            _ => {
                // Unhandled (already checked above — should not reach here).
            }
        }
    }

    // ── Emit final fragment ────────────────────────────────────────────────

    // WITH clause (if any)
    if let Some(ref wc) = with_clause {
        print_node(wc, ctx, out);
        out.push(' ');
    }

    let final_sql = build_select_fragment_with_having(
        &inner_source,
        &acc_where,
        &acc_select,
        &acc_order_by,
        &acc_limit,
        acc_distinct,
        &acc_group_by,
        &acc_having,
    );
    out.push_str(&final_sql);
}

/// Build a SELECT fragment from accumulated state (with optional GROUP BY and HAVING).
///
/// When `group_by_body` is `Some`, emits `GROUP BY <group_by_body>` after the WHERE.
/// When `having_bodies` is non-empty (requires `group_by_body` to be meaningful), emits
/// `HAVING <having_bodies joined with AND>`.
#[allow(clippy::too_many_arguments)]
fn build_select_fragment_with_having(
    inner_source: &str,
    where_bodies: &[String],
    select_body: &Option<String>,
    order_by_body: &Option<String>,
    limit_body: &Option<String>,
    has_distinct: bool,
    group_by_body: &Option<String>,
    having_bodies: &[String],
) -> String {
    let mut s = String::new();

    s.push_str("SELECT");
    if has_distinct {
        s.push_str(" DISTINCT");
    }
    s.push(' ');

    match select_body {
        Some(body) => s.push_str(body),
        None => s.push('*'),
    }

    s.push_str(" FROM ");
    s.push_str(inner_source);

    if !where_bodies.is_empty() {
        s.push_str(" WHERE ");
        s.push_str(&where_bodies.join(" AND "));
    }

    if let Some(gb) = group_by_body {
        s.push_str(" GROUP BY ");
        s.push_str(gb);
    }

    if !having_bodies.is_empty() {
        s.push_str(" HAVING ");
        s.push_str(&having_bodies.join(" AND "));
    }

    if let Some(ob) = order_by_body {
        s.push_str(" ORDER BY ");
        s.push_str(ob);
    }

    if let Some(lim) = limit_body {
        s.push_str(" LIMIT ");
        s.push_str(lim);
    }

    s
}

/// Collect the FROM body (everything after the FROM keyword) as a printed string.
fn collect_from_body(from_clause: &SyntaxNode, ctx: &PrintContext) -> String {
    use SyntaxKind::*;

    let mut body = String::new();
    let mut past_from_kw = false;
    for child_elem in from_clause.children_with_tokens() {
        match child_elem {
            SyntaxElement::Token(t) => {
                if t.kind() == FROM_KW {
                    past_from_kw = true;
                    continue;
                }
                if past_from_kw {
                    body.push_str(t.text());
                }
            }
            SyntaxElement::Node(n) => {
                if past_from_kw {
                    print_node(&n, ctx, &mut body);
                }
            }
        }
    }
    body.trim().to_string()
}

/// Collect the body text of a stage after its operator marker, running nodes
/// through `print_node` for smelt-path expansion.
///
/// The CST for contextual-keyword stages (EXTEND, SET, DROP, RENAME) has:
///   PIPE_STAGE { PIPE_OP_<X> [zero-width marker]  IDENT("<keyword>")  ... body ... }
///
/// We set `past_op` after the zero-width marker node, then skip one more IDENT
/// that is the operator keyword itself (e.g. "EXTEND", "SET"), and also any
/// immediately following whitespace, before appending body content.
fn collect_stage_body_text(stage: &SyntaxNode, ctx: &PrintContext, op_kind: SyntaxKind) -> String {
    use SyntaxKind::*;

    let mut past_op = false;
    let mut skipped_keyword = false;
    let mut body = String::new();

    for elem in stage.children_with_tokens() {
        match &elem {
            SyntaxElement::Node(n) => {
                if n.kind() == op_kind {
                    past_op = true;
                    continue;
                }
                if past_op {
                    print_node(n, ctx, &mut body);
                }
            }
            SyntaxElement::Token(t) => {
                if !past_op {
                    continue;
                }
                if !skipped_keyword {
                    // Skip the operator keyword IDENT (e.g. "EXTEND", "SET").
                    if t.kind() == IDENT {
                        skipped_keyword = true;
                        continue;
                    }
                    // Skip trivia (whitespace) before we've seen the keyword IDENT.
                    if t.kind().is_trivia() {
                        continue;
                    }
                    // Anything else: keyword was implicit/absent, start body here.
                    skipped_keyword = true;
                    body.push_str(t.text());
                } else {
                    body.push_str(t.text());
                }
            }
        }
    }
    body.trim().to_string()
}

/// Extract the alias name from a PIPE_OP_AS stage.
fn collect_as_alias(stage: &SyntaxNode) -> Option<String> {
    use SyntaxKind::*;
    let mut past_op = false;
    for elem in stage.children_with_tokens() {
        match &elem {
            SyntaxElement::Node(n) if n.kind() == PIPE_OP_AS => {
                past_op = true;
            }
            SyntaxElement::Token(t) if past_op && !t.kind().is_trivia() && t.kind() == IDENT => {
                return Some(t.text().to_string());
            }
            _ => {}
        }
    }
    None
}

/// Collect the body text for a WHERE pipe stage.
///
/// The PIPE_STAGE for WHERE contains:
/// - `PIPE_OP_WHERE` marker (zero-width node)
/// - `WHERE_KW` token
/// - whitespace
/// - `EXPRESSION` node (the predicate)
///
/// We skip the marker and the WHERE keyword and return the predicate text.
fn collect_where_body(stage: &SyntaxNode) -> String {
    use SyntaxKind::*;

    let mut past_op = false;
    let mut body = String::new();

    for elem in stage.children_with_tokens() {
        match &elem {
            SyntaxElement::Node(n) => {
                let k = n.kind();
                if matches!(k, PIPE_OP_WHERE) {
                    // Skip zero-width marker
                    continue;
                }
                // Any other node (EXPRESSION, etc.) is body content.
                past_op = true;
                body.push_str(&n.text().to_string());
            }
            SyntaxElement::Token(t) => {
                if !past_op {
                    // Skip WHERE_KW and whitespace before the predicate.
                    if matches!(t.kind(), WHERE_KW | WHITESPACE) {
                        continue;
                    }
                    // First non-keyword, non-whitespace token → body starts.
                    past_op = true;
                }
                body.push_str(t.text());
            }
        }
    }

    body.trim().to_string()
}

/// Collect the SELECT list body for a SELECT pipe stage.
///
/// The PIPE_STAGE for SELECT contains:
/// - `PIPE_OP_SELECT` marker (zero-width node)
/// - `SELECT_KW` token
/// - whitespace
/// - `SELECT_LIST` node (the projection list)
///
/// We skip the marker and SELECT keyword and return the list text.
fn collect_select_body(stage: &SyntaxNode) -> String {
    use SyntaxKind::*;

    let mut past_kw = false;
    let mut body = String::new();

    for elem in stage.children_with_tokens() {
        match &elem {
            SyntaxElement::Node(n) => {
                let k = n.kind();
                if matches!(k, PIPE_OP_SELECT) {
                    continue;
                }
                // SELECT_LIST or any other node → body
                past_kw = true;
                body.push_str(&n.text().to_string());
            }
            SyntaxElement::Token(t) => {
                if !past_kw {
                    if matches!(t.kind(), SELECT_KW | WHITESPACE) {
                        continue;
                    }
                    past_kw = true;
                }
                body.push_str(t.text());
            }
        }
    }

    body.trim().to_string()
}

/// Collect the ORDER BY body for an ORDER BY pipe stage.
///
/// The PIPE_STAGE for ORDER BY contains:
/// - `PIPE_OP_ORDER_BY` marker (zero-width node)
/// - `ORDER_BY_CLAUSE` node (which itself starts with ORDER_KW BY_KW)
///
/// We emit the text of the ORDER_BY_CLAUSE *after* stripping "ORDER BY " prefix.
fn collect_order_by_body(stage: &SyntaxNode) -> String {
    use SyntaxKind::*;

    // Find the ORDER_BY_CLAUSE child node.
    for child in stage.children() {
        if child.kind() == ORDER_BY_CLAUSE {
            // The ORDER_BY_CLAUSE text starts with "ORDER BY …".
            // We want just the items list (everything after ORDER BY).
            // Strip leading "ORDER BY " (case-insensitive, may have varying whitespace).
            // We do this by scanning past the ORDER + BY tokens.
            let mut past_by = false;
            let mut body = String::new();
            for elem in child.children_with_tokens() {
                match &elem {
                    SyntaxElement::Token(t) => {
                        if !past_by {
                            if matches!(t.kind(), ORDER_KW | BY_KW | WHITESPACE) {
                                if t.kind() == BY_KW {
                                    past_by = true;
                                }
                                continue;
                            }
                            past_by = true;
                        }
                        body.push_str(t.text());
                    }
                    SyntaxElement::Node(n) => {
                        if past_by {
                            body.push_str(&n.text().to_string());
                        }
                    }
                }
            }
            return body.trim().to_string();
        }
    }

    // Fallback: return raw text if no ORDER_BY_CLAUSE found.
    stage.text().to_string().trim().to_string()
}

/// Collect the aggregate expressions and group-by expressions from a PIPE_OP_AGGREGATE stage.
///
/// Returns `(agg_body, group_by_body)` where:
/// - `agg_body` is the comma-separated aggregate expression list (with aliases), as printed SQL.
/// - `group_by_body` is the comma-separated group-by expression list (with aliases), as printed SQL.
///   Empty string when there is no GROUP BY clause (full-table aggregation).
///
/// CST structure for `|> AGGREGATE sum(x) AS s GROUP BY k`:
///   PIPE_STAGE {
///     PIPE_OP_AGGREGATE (zero-width node)
///     IDENT("AGGREGATE")
///     EXPRESSION(sum(x))
///     AS_KW
///     IDENT("s")
///     GROUP_KW
///     BY_KW
///     EXPRESSION(k)
///   }
fn collect_aggregate_parts(stage: &SyntaxNode, ctx: &PrintContext) -> (String, String) {
    use SyntaxKind::*;

    let children: Vec<SyntaxElement> = stage.children_with_tokens().collect();

    // Find GROUP_KW position (if any) to split agg vs group-by portions.
    let group_kw_pos = children
        .iter()
        .position(|elem| matches!(elem, SyntaxElement::Token(t) if t.kind() == GROUP_KW));

    // Determine end of agg portion (exclusive).
    let agg_end = group_kw_pos.unwrap_or(children.len());

    // Find where the agg portion starts: past PIPE_OP_AGGREGATE node and the "AGGREGATE" IDENT.
    let mut agg_start = 0;
    // Skip PIPE_OP_AGGREGATE zero-width node.
    for (i, elem) in children.iter().enumerate() {
        if let SyntaxElement::Node(n) = elem {
            if n.kind() == PIPE_OP_AGGREGATE {
                agg_start = i + 1;
                break;
            }
        }
    }
    // Skip trivia and then the "AGGREGATE" contextual keyword IDENT.
    while agg_start < agg_end {
        match &children[agg_start] {
            SyntaxElement::Token(t) if t.kind().is_trivia() => agg_start += 1,
            SyntaxElement::Token(t) if t.kind() == IDENT => {
                agg_start += 1;
                break;
            }
            _ => break,
        }
    }

    // Collect agg portion as printed SQL text.
    let agg_body = collect_elements_as_text(&children[agg_start..agg_end], ctx)
        .trim()
        .to_string();

    // Collect group-by portion (after BY_KW).
    let group_body = if let Some(gkw) = group_kw_pos {
        // Find BY_KW after GROUP_KW.
        let by_pos = children[gkw..]
            .iter()
            .position(|elem| matches!(elem, SyntaxElement::Token(t) if t.kind() == BY_KW));
        if let Some(by_rel) = by_pos {
            let by_abs = gkw + by_rel + 1; // +1 to skip past BY_KW
            collect_elements_as_text(&children[by_abs..], ctx)
                .trim()
                .to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    (agg_body, group_body)
}

/// Print a slice of CST elements as SQL text, running nodes through `print_node`.
fn collect_elements_as_text(elements: &[SyntaxElement], ctx: &PrintContext) -> String {
    let mut out = String::new();
    for elem in elements {
        match elem {
            SyntaxElement::Token(t) => out.push_str(t.text()),
            SyntaxElement::Node(n) => print_node(n, ctx, &mut out),
        }
    }
    out
}

/// Collect the LIMIT value for a LIMIT pipe stage.
///
/// The PIPE_STAGE for LIMIT contains:
/// - `PIPE_OP_LIMIT` marker (zero-width node)
/// - `LIMIT_CLAUSE` node (which itself contains LIMIT_KW and the value)
///
/// We emit the text of the LIMIT_CLAUSE after stripping "LIMIT " prefix.
fn collect_limit_body(stage: &SyntaxNode) -> String {
    use SyntaxKind::*;

    for child in stage.children() {
        if child.kind() == LIMIT_CLAUSE {
            // Strip the LIMIT keyword, return just the value (and optional OFFSET).
            let mut past_kw = false;
            let mut body = String::new();
            for elem in child.children_with_tokens() {
                match &elem {
                    SyntaxElement::Token(t) => {
                        if !past_kw {
                            if matches!(t.kind(), LIMIT_KW | WHITESPACE) {
                                if t.kind() == LIMIT_KW {
                                    past_kw = true;
                                }
                                continue;
                            }
                            past_kw = true;
                        }
                        body.push_str(t.text());
                    }
                    SyntaxElement::Node(n) => {
                        if past_kw {
                            body.push_str(&n.text().to_string());
                        }
                    }
                }
            }
            return body.trim().to_string();
        }
    }

    stage.text().to_string().trim().to_string()
}

/// Reparse an expanded `smelt.define` body so any nested `SMELT_PATH_CALL`
/// nodes are recognised, then print it through `ctx` to re-expand nested
/// calls.
///
/// The smelt parser only produces `SMELT_PATH_CALL` nodes inside a
/// statement context (reachable from `SELECT`), never inside a bare or
/// parenthesised fragment.  Wrapping the body in a synthetic `SELECT `
/// prefix forces the parser into statement context so nested path-calls
/// are parsed and subsequently re-expanded by `print_node`.  The synthetic
/// prefix is stripped from the returned string before returning.
///
/// This makes nested/transitive `smelt.functions.*` chains reach a
/// fixpoint: each `print_node` invocation expands one level; the recursion
/// terminates because `FunctionCallCycle` rejects all circular definitions
/// before the build reaches this point.
fn reexpand_call_body(expanded: &str, ctx: &PrintContext) -> String {
    let wrapped = format!("SELECT {expanded}");
    let reparsed = smelt_parser::parse(&wrapped);
    let mut out = String::new();
    print_node(&reparsed.syntax(), ctx, &mut out);
    out.strip_prefix("SELECT ")
        .expect("synthetic SELECT wrapper prefix is always present (print_node on a FILE starts with the SELECT keyword)")
        .to_string()
}

/// Walk children with index-based iteration, allowing look-ahead for DATE literal rewrite.
fn print_children(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    let children: Vec<SyntaxElement> = node.children_with_tokens().collect();
    let mut i = 0;
    while i < children.len() {
        match &children[i] {
            SyntaxElement::Token(token) => {
                // DATE literal rewrite: DATE 'value' → DATE('value')
                if !ctx.capabilities.supports_date_literal
                    && token.kind() == SyntaxKind::IDENT
                    && token.text().eq_ignore_ascii_case("DATE")
                {
                    if let Some((skip_to, string_text)) = find_string_after(&children, i + 1) {
                        out.push_str("DATE(");
                        out.push_str(&string_text);
                        out.push(')');
                        i = skip_to + 1;
                        continue;
                    }
                }
                out.push_str(token.text());
            }
            SyntaxElement::Node(child_node) => {
                print_node(child_node, ctx, out);
            }
        }
        i += 1;
    }
}

/// Look ahead in children for optional whitespace followed by a STRING token.
/// Returns (index_of_string, string_text) if found.
fn find_string_after(children: &[SyntaxElement], start: usize) -> Option<(usize, String)> {
    let mut j = start;
    while j < children.len() {
        match &children[j] {
            SyntaxElement::Token(t) if t.kind() == SyntaxKind::WHITESPACE => {
                j += 1;
            }
            SyntaxElement::Token(t) if t.kind() == SyntaxKind::STRING => {
                return Some((j, t.text().to_string()));
            }
            _ => return None,
        }
    }
    None
}

/// Rewrite expr::type → CAST(expr AS type) when backend doesn't support ::.
/// If it's already CAST(...) syntax, pass through verbatim.
fn print_cast_rewrite(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    let Some(cast) = CastExpr::cast(node.clone()) else {
        print_children(node, ctx, out);
        return;
    };

    if !cast.is_double_colon_cast() {
        // Already CAST(expr AS type) syntax — pass through
        print_children(node, ctx, out);
        return;
    }

    // Partition children into: expr (before ::), type (TYPE_SPEC node), trailing whitespace.
    // We emit CAST(expr AS type) followed by any trailing whitespace.
    let children: Vec<SyntaxElement> = node.children_with_tokens().collect();

    // Find the :: token index
    let dc_idx = children
        .iter()
        .position(|c| matches!(c, SyntaxElement::Token(t) if t.kind() == SyntaxKind::DOUBLE_COLON));
    let Some(dc_idx) = dc_idx else {
        print_children(node, ctx, out);
        return;
    };

    // Find the TYPE_SPEC node index
    let type_idx = children
        .iter()
        .position(|c| matches!(c, SyntaxElement::Node(n) if n.kind() == SyntaxKind::TYPE_SPEC));

    out.push_str("CAST(");

    // Print expression (children before ::)
    for child in &children[..dc_idx] {
        match child {
            SyntaxElement::Token(t) => out.push_str(t.text()),
            SyntaxElement::Node(n) => print_node(n, ctx, out),
        }
    }

    out.push_str(" AS ");

    // Print TYPE_SPEC, moving any trailing whitespace outside the closing paren
    let mut type_text = String::new();
    if let Some(ti) = type_idx {
        if let SyntaxElement::Node(n) = &children[ti] {
            print_node(n, ctx, &mut type_text);
        }
    }
    let trimmed = type_text.trim_end();
    let trailing = &type_text[trimmed.len()..];
    out.push_str(trimmed);
    out.push(')');
    out.push_str(trailing);

    // Print any remaining children after TYPE_SPEC (unlikely but defensive)
    let after = type_idx.map(|ti| ti + 1).unwrap_or(children.len());
    for child in &children[after..] {
        match child {
            SyntaxElement::Token(t) => out.push_str(t.text()),
            SyntaxElement::Node(n) => print_node(n, ctx, out),
        }
    }
}

/// Expand a `SMELT_PATH_CALL_STAR` node (`smelt.<path>(args).*`) to
/// per-field projections `<expr> AS <name>, …`.
///
/// Steps:
/// 1. Find the inner `SMELT_PATH_CALL` child and ask the expander for the body.
/// 2. Re-parse the body and find a top-level `BRACE_STRUCT_LITERAL`.
/// 3. Emit each `STRUCT_FIELD_ITEM` verbatim (already `<expr> AS <name>` form).
///
/// Returns `None` when any step fails (no expander, expander returned `None`,
/// body has no brace-struct literal, or body has a `SPREAD_ITEM`), so the
/// caller falls back to verbatim printing.
fn expand_smelt_path_call_star(node: &SyntaxNode, ctx: &PrintContext) -> Option<String> {
    let expander = ctx.smelt_path_call.as_ref()?;

    // Locate the SMELT_PATH_CALL child node.
    let call_node = node
        .children()
        .find(|c| c.kind() == SyntaxKind::SMELT_PATH_CALL)?;

    let path_call = SmeltPathCall::cast(call_node.clone())?;
    let segs = path_call.segments();
    let positional: Vec<String> = path_call
        .arg_list()
        .map(|al| {
            al.positional_args()
                .into_iter()
                .map(|arg| {
                    let mut s = String::new();
                    print_node(arg.syntax(), ctx, &mut s);
                    s
                })
                .collect()
        })
        .unwrap_or_default();
    let named: Vec<(String, String)> = path_call
        .arg_list()
        .map(|al| {
            al.named_params()
                .filter_map(|np| {
                    let name = np.name()?;
                    let expr = np.value_expr()?;
                    let mut s = String::new();
                    print_node(expr.syntax(), ctx, &mut s);
                    Some((name, s))
                })
                .collect()
        })
        .unwrap_or_default();

    let body = expander(&segs, positional, named)?;

    // Re-parse the body to locate a BRACE_STRUCT_LITERAL.
    //
    // The body is an expression fragment (e.g. `{expr AS name, …}`), not a
    // full SQL statement.  `smelt_parser::parse` calls `parse_file`, which
    // only recognises SELECT/WITH/smelt.define at the top level, so a bare
    // brace-struct literal `{…}` would hit the error path and not produce a
    // `BRACE_STRUCT_LITERAL` node.  Wrapping the body in a minimal
    // `SELECT <body>` forces expression parsing and places the struct literal
    // inside a `SELECT_ITEM → EXPRESSION` wrapper where the traversal below
    // can find it.
    let wrapper = format!("SELECT {body}");
    let reparsed = smelt_parser::parse(&wrapper);
    let syntax_root = reparsed.syntax();

    // Walk to find the BRACE_STRUCT_LITERAL node anywhere in the tree.
    let brace_struct = find_brace_struct_literal(&syntax_root)?;

    // Reject if any SPREAD_ITEM is present — fall back to verbatim.
    let has_spread = brace_struct
        .children()
        .any(|c| c.kind() == SyntaxKind::SPREAD_ITEM);
    if has_spread {
        return None;
    }

    // Collect STRUCT_FIELD_ITEM children and emit them as separate projections.
    let fields: Vec<SyntaxNode> = brace_struct
        .children()
        .filter(|c| c.kind() == SyntaxKind::STRUCT_FIELD_ITEM)
        .collect();

    if fields.is_empty() {
        return None;
    }

    let mut out = String::new();
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        // Each STRUCT_FIELD_ITEM is `<expr> AS <name>` — print it verbatim
        // (running it through print_node applies further expansions if needed).
        print_node(field, ctx, &mut out);
    }
    Some(out)
}

/// Check whether the `TABLE_REF` parent of a `SMELT_PATH_CALL` carries a
/// user-supplied alias.
///
/// The parser places a `SMELT_PATH_CALL` inside `TABLE_REF` and then, if the
/// user wrote `AS alias` or a bare implicit alias, appends those tokens as
/// siblings in the same `TABLE_REF` node.  We scan the children of
/// `table_ref` and look for any `AS_KW` or `IDENT` token that appears after
/// the `SMELT_PATH_CALL` child.
fn smelt_path_call_has_explicit_alias(table_ref: &SyntaxNode, call_node: &SyntaxNode) -> bool {
    let call_range = call_node.text_range();
    let mut past_call = false;
    for child in table_ref.children_with_tokens() {
        match &child {
            SyntaxElement::Node(n) => {
                if n.text_range() == call_range {
                    past_call = true;
                }
            }
            SyntaxElement::Token(t) => {
                if past_call && !t.kind().is_trivia() {
                    // Any non-trivia token after the SMELT_PATH_CALL is either
                    // AS_KW (explicit alias) or IDENT (implicit alias).  Both
                    // indicate a user-supplied alias.
                    if matches!(t.kind(), SyntaxKind::AS_KW | SyntaxKind::IDENT) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// Recursively find the first `BRACE_STRUCT_LITERAL` node in the tree,
/// descending through FILE and EXPRESSION wrapper nodes.
fn find_brace_struct_literal(node: &SyntaxNode) -> Option<SyntaxNode> {
    if node.kind() == SyntaxKind::BRACE_STRUCT_LITERAL {
        return Some(node.clone());
    }
    for child in node.children() {
        if let Some(found) = find_brace_struct_literal(&child) {
            return Some(found);
        }
    }
    None
}

/// Rewrite ARRAY[1,2,3] → ARRAY(1,2,3).
fn print_array_rewrite(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    for child in node.children_with_tokens() {
        match child {
            SyntaxElement::Token(token) => match token.kind() {
                SyntaxKind::LBRACKET => out.push('('),
                SyntaxKind::RBRACKET => out.push(')'),
                _ => out.push_str(token.text()),
            },
            SyntaxElement::Node(child_node) => {
                print_node(&child_node, ctx, out);
            }
        }
    }
}

/// Handle SELECT with QUALIFY → subquery rewrite when backend doesn't support QUALIFY.
fn print_select_with_qualify_rewrite(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    let has_qualify = node
        .children()
        .any(|c| c.kind() == SyntaxKind::QUALIFY_CLAUSE);

    if !has_qualify {
        print_children(node, ctx, out);
        return;
    }

    // Extract the QUALIFY expression
    let qualify_expr = node
        .children()
        .find(|c| c.kind() == SyntaxKind::QUALIFY_CLAUSE)
        .and_then(|qc| {
            let mut found_kw = false;
            let mut expr_parts = Vec::new();
            for child in qc.children_with_tokens() {
                match child {
                    SyntaxElement::Token(t) => {
                        if t.kind() == SyntaxKind::QUALIFY_KW {
                            found_kw = true;
                        } else if found_kw {
                            expr_parts.push(t.text().to_string());
                        }
                    }
                    SyntaxElement::Node(n) => {
                        if found_kw {
                            let mut s = String::new();
                            print_node(&n, ctx, &mut s);
                            expr_parts.push(s);
                        }
                    }
                }
            }
            let trimmed = expr_parts.join("").trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });

    let Some(qualify_expr) = qualify_expr else {
        print_children(node, ctx, out);
        return;
    };

    // Wrap: SELECT * FROM (inner_select_without_qualify) _q WHERE qualify_expr
    out.push_str("SELECT * FROM (");
    print_children_skip_qualify(node, ctx, out);
    out.push_str(") _q WHERE ");
    out.push_str(&qualify_expr);
}

/// Print a SELECT statement's children, skipping the QUALIFY clause.
fn print_children_skip_qualify(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    let children: Vec<SyntaxElement> = node.children_with_tokens().collect();
    let mut i = 0;
    while i < children.len() {
        match &children[i] {
            SyntaxElement::Token(token) => {
                if !ctx.capabilities.supports_date_literal
                    && token.kind() == SyntaxKind::IDENT
                    && token.text().eq_ignore_ascii_case("DATE")
                {
                    if let Some((skip_to, string_text)) = find_string_after(&children, i + 1) {
                        out.push_str("DATE(");
                        out.push_str(&string_text);
                        out.push(')');
                        i = skip_to + 1;
                        continue;
                    }
                }
                out.push_str(token.text());
            }
            SyntaxElement::Node(child_node) => {
                if child_node.kind() == SyntaxKind::QUALIFY_CLAUSE {
                    i += 1;
                    continue;
                }
                print_node(child_node, ctx, out);
            }
        }
        i += 1;
    }
}

/// Print children of a SELECT_LIST or GROUP_BY_CLAUSE, stripping trailing commas.
fn print_strip_trailing_commas(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    let children: Vec<SyntaxElement> = node.children_with_tokens().collect();
    for (i, child) in children.iter().enumerate() {
        match child {
            SyntaxElement::Token(token) if token.kind() == SyntaxKind::COMMA => {
                // Look ahead: is there any non-whitespace child after this comma?
                let has_more = children[i + 1..].iter().any(
                    |c| !matches!(c, SyntaxElement::Token(t) if t.kind() == SyntaxKind::WHITESPACE),
                );
                if has_more {
                    out.push_str(token.text());
                }
                // else: trailing comma — skip it (but keep any following whitespace)
            }
            SyntaxElement::Token(token) => {
                out.push_str(token.text());
            }
            SyntaxElement::Node(child_node) => {
                print_node(child_node, ctx, out);
            }
        }
    }
}

/// Parse a SET stage body `col = expr, col2 = expr2` into `[(col, expr), ...]`.
///
/// Splits on commas that are not nested inside parentheses, then for each item
/// splits on the first `=` to extract the column name (left-hand side) and
/// expression (right-hand side).  Whitespace is trimmed from both sides.
///
/// If any item lacks a `=`, it is skipped — the caller emits the whole body
/// verbatim in that case (but currently the `has_unhandled` check rejects
/// syntactically broken SET stages before we get here).
fn parse_set_assignments(body: &str) -> Vec<(String, String)> {
    split_comma_top_level(body)
        .into_iter()
        .filter_map(|item| {
            let item = item.trim();
            let eq_pos = item.find('=')?;
            let col = item[..eq_pos].trim().to_string();
            let expr = item[eq_pos + 1..].trim().to_string();
            if col.is_empty() || expr.is_empty() {
                None
            } else {
                Some((col, expr))
            }
        })
        .collect()
}

/// Parse a DROP stage body `col1, col2, ...` into `["col1", "col2", ...]`.
fn parse_column_list(body: &str) -> Vec<String> {
    split_comma_top_level(body)
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Parse a RENAME stage body `old AS new, old2 AS new2` into `[(old, new), ...]`.
///
/// Each item must contain a case-insensitive ` AS ` separator.  Items that lack
/// it are skipped.
fn parse_rename_pairs(body: &str) -> Vec<(String, String)> {
    split_comma_top_level(body)
        .into_iter()
        .filter_map(|item| {
            let item = item.trim();
            // Find " AS " (case-insensitive).
            let upper = item.to_uppercase();
            let as_pos = upper.find(" AS ")?;
            let old = item[..as_pos].trim().to_string();
            let new = item[as_pos + 4..].trim().to_string();
            if old.is_empty() || new.is_empty() {
                None
            } else {
                Some((old, new))
            }
        })
        .collect()
}

/// Split a comma-separated list at the top level (not inside parentheses).
///
/// Tracks paren depth so that expressions like `COALESCE(a, b)` or
/// `DATE_TRUNC('month', ts)` are not split on their internal commas.
fn split_comma_top_level(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth: u32 = 0;
    let mut current = String::new();
    for ch in s.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                parts.push(current.clone());
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts
}

/// Lower `MEDIAN(x)` to GoogleSQL, which has no `MEDIAN` built-in.
///
/// Returns `true` when the call was printed here, `false` to fall through to
/// the normal path (leaving the call verbatim so the engine rejects it loudly
/// rather than smelt guessing).
///
/// Both forms below are **exact**, which is the whole constraint: substituting
/// an approximate median under the equivalence oracle would make it report
/// divergences that are artefacts of the substitution, or hide real ones.
/// `APPROX_QUANTILES` is an aggregate but approximate, so it is not a candidate.
/// Measured against the live warehouse by `scripts/bigquery-probe-lowering.sh`
/// on the fixture `val ∈ {1,2,3,4}` — an even count, where an interpolating
/// median (2.5) and a nearest-rank one (2) differ:
///
/// - **Window position** (`MEDIAN(x) OVER w`) → `PERCENTILE_CONT(x, 0.5) OVER w`.
///   Exact and interpolating; measured 2.5, agreeing with DuckDB's `MEDIAN`.
///   `PERCENTILE_DISC` returns 2 and is therefore *not* the equivalent.
/// - **Aggregate position** (`SELECT MEDIAN(x) … GROUP BY …`) →
///   an `ARRAY_AGG`-indexing expression. `PERCENTILE_CONT` is analytic-only in
///   GoogleSQL and is rejected outright in a `GROUP BY` query, so it cannot
///   stand here. Binding the array to a name via `UNNEST([ARRAY_AGG(…)])` is
///   also rejected (`Aggregate function ARRAY_AGG not allowed in UNNEST`), which
///   is why the sub-expression is repeated rather than named. Measured on
///   grouped fixtures against DuckDB's answers: even count → 2.5, odd → 2,
///   NULLs ignored → 1.5, all-NULL group → NULL.
///
/// The aggregate form casts to `FLOAT64`, matching DuckDB's `MEDIAN` return
/// type for numeric input. A temporal argument — which DuckDB's `MEDIAN` also
/// accepts — makes BigQuery reject the cast, i.e. it fails loud rather than
/// returning a wrong value.
fn print_bigquery_median(
    node: &SyntaxNode,
    fc: &FunctionCall,
    position: Position,
    ctx: &PrintContext,
    out: &mut String,
) -> bool {
    let args = fc.arguments();
    let [arg] = args.as_slice() else {
        return false;
    };
    let mut arg_sql = String::new();
    print_node(arg.syntax(), ctx, &mut arg_sql);
    let arg_sql = arg_sql.trim();
    if arg_sql.is_empty() {
        return false;
    }

    // The call's position is decided once, by the compile path, from the
    // source CST (`position::classify`) — this function is never handed
    // anything else to derive it from.
    let windowed = matches!(position, Position::WholePartitionWindow | Position::Window);
    if windowed {
        out.push_str(&format!("PERCENTILE_CONT({arg_sql}, 0.5)"));
        push_trailing_trivia(node, out);
        return true;
    }

    let sorted = format!("ARRAY_AGG({arg_sql} IGNORE NULLS ORDER BY {arg_sql})");
    let mid = format!("DIV(ARRAY_LENGTH({sorted}), 2)");
    let at = |index: &str| format!("CAST({sorted}[SAFE_OFFSET({index})] AS FLOAT64)");
    out.push_str(&format!(
        "CASE WHEN MOD(ARRAY_LENGTH({sorted}), 2) = 1 THEN {upper} \
         ELSE ({lower} + {upper}) / 2 END",
        upper = at(&mid),
        lower = at(&format!("{mid} - 1")),
    ));
    push_trailing_trivia(node, out);
    true
}

/// Lower infix `%` (modulo) to `MOD(x, y)`.
///
/// The registry establishes that this operator needs a `ModuloCall` rewrite for
/// the target dialect before this function is called; the operator identity check
/// is not repeated here.
///
/// Returns `true` when the expression was printed here (a `%` `BINARY_EXPR`
/// with both operands present), `false` to fall through to the normal path
/// so every other `BINARY_EXPR` (`+`, `-`, `AND`, …) still prints verbatim.
fn print_modulo_call(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) -> bool {
    let Some(bin) = BinaryExpr::cast(node.clone()) else {
        return false;
    };
    let (Some(left), Some(right)) = (bin.left(), bin.right()) else {
        return false;
    };
    let mut left_sql = String::new();
    print_node(left.syntax(), ctx, &mut left_sql);
    let mut right_sql = String::new();
    print_node(right.syntax(), ctx, &mut right_sql);
    let left_sql = left_sql.trim();
    let right_sql = right_sql.trim();
    if left_sql.is_empty() || right_sql.is_empty() {
        return false;
    }

    out.push_str(&format!("MOD({left_sql}, {right_sql})"));
    push_trailing_trivia(node, out);
    true
}

/// Lower infix `^` and `**` (power) to `POWER(x, y)`.
///
/// Both operators are DuckDB-exact synonyms for `POWER(x, y)` (measured against
/// DuckDB 1.5.4: `2 ^ 10`, `2 ** 10`, and `power(2, 10)` all print `1024.0`).
/// On Spark, `^` is bitwise XOR — a silent-wrong-answer hazard, not a syntax
/// error, if left unlowered. The registry establishes which dialects need this
/// rewrite before this function is called; the operator identity check is not
/// repeated here.
///
/// Returns `true` when the expression was printed here (a `^`/`**`
/// `BINARY_EXPR` with both operands present), `false` to fall through so
/// every other `BINARY_EXPR` still prints verbatim.
fn print_power_call(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) -> bool {
    let Some(bin) = BinaryExpr::cast(node.clone()) else {
        return false;
    };
    let (Some(left), Some(right)) = (bin.left(), bin.right()) else {
        return false;
    };
    let mut left_sql = String::new();
    print_node(left.syntax(), ctx, &mut left_sql);
    let mut right_sql = String::new();
    print_node(right.syntax(), ctx, &mut right_sql);
    let left_sql = left_sql.trim();
    let right_sql = right_sql.trim();
    if left_sql.is_empty() || right_sql.is_empty() {
        return false;
    }

    out.push_str(&format!("POWER({left_sql}, {right_sql})"));
    push_trailing_trivia(node, out);
    true
}

/// Re-emit the trivia tokens trailing a node whose text a rewrite replaced,
/// so a following sibling (an `OVER` clause, the next select item) does not
/// end up glued to the rewritten text.
fn push_trailing_trivia(node: &SyntaxNode, out: &mut String) {
    let tokens: Vec<_> = node
        .children_with_tokens()
        .filter_map(|e| e.into_token())
        .collect();
    let trailing: Vec<_> = tokens
        .into_iter()
        .rev()
        .take_while(|t| t.kind().is_trivia())
        .collect();
    for token in trailing.into_iter().rev() {
        out.push_str(token.text());
    }
}

/// Resolve the call's registry entry and dispatch on its emission verdict for
/// this dialect. Returns `true` when the node was fully printed.
///
/// `BuiltinRegistry::resolve` folds case; `FunctionCall::name()` returns the raw
/// source spelling, preserving whatever casing the author used.
fn emit_registered_function(
    node: &SyntaxNode,
    fc: &FunctionCall,
    name: &str,
    ctx: &PrintContext,
    out: &mut String,
) -> bool {
    let Some(sig) = BuiltinRegistry::resolve(name) else {
        return false;
    };
    // Position is decided once, from the source CST, and handed to the
    // registry — the printer never re-derives it (`docs/specs/multi_backend.md`
    // §"Emission is scoped to call position"). The topmost ancestor of `node`
    // is the root the classifier resolves named `WINDOW` clauses against.
    let root = node.ancestors().last().unwrap_or_else(|| node.clone());
    let position = classify_position(node, &root);
    match sig.emission_at(ctx.dialect.id(), position) {
        // An `Unsupported` entry still prints verbatim; the compile path refuses
        // the model before reaching the printer (see `emission_check`), so a
        // verbatim print here is unreachable in production and harmless in a
        // printer unit test.
        Emission::Native | Emission::Unsupported { .. } => false,
        Emission::Rename(new_name) => {
            // The author already wrote the target spelling — via an alias, or
            // in different case. Rewriting it would churn the user's own text
            // (and break DuckDB byte-identity, `architecture.md` §"Print-level
            // identity for the DuckDB dialect": input already using
            // DuckDB-flavoured spellings round-trips byte-identically).
            if name.eq_ignore_ascii_case(new_name) {
                return false;
            }
            print_function_with_renamed(node, ctx, out, new_name);
            true
        }
        Emission::Rewrite(id) => apply_rewrite(id, node, Some(fc), position, ctx, out),
    }
}

fn emit_registered_operator(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) -> bool {
    let Some(op) = BinaryExpr::cast(node.clone()).and_then(|b| b.operator()) else {
        return false;
    };
    let Some(sig) = BuiltinRegistry::resolve(&op) else {
        return false;
    };
    // Operators are never a call in window/aggregate position; their verdicts
    // are stated with `Position::Any`, so there is no position to classify.
    match sig.emission_at(ctx.dialect.id(), Position::Any) {
        Emission::Rewrite(id) => apply_rewrite(id, node, None, Position::Any, ctx, out),
        _ => false,
    }
}

/// The one place a `RewriteId` becomes code. Adding a variant is a compile error
/// here until it is implemented.
fn apply_rewrite(
    id: RewriteId,
    node: &SyntaxNode,
    fc: Option<&FunctionCall>,
    position: Position,
    ctx: &PrintContext,
    out: &mut String,
) -> bool {
    match id {
        RewriteId::BigQueryMedian => {
            fc.is_some_and(|fc| print_bigquery_median(node, fc, position, ctx, out))
        }
        RewriteId::ModuloCall => print_modulo_call(node, ctx, out),
        RewriteId::PowerCall => print_power_call(node, ctx, out),
    }
}

fn print_function_with_renamed(
    node: &SyntaxNode,
    ctx: &PrintContext,
    out: &mut String,
    new_name: &str,
) {
    // For a simple function call, the first IDENT token is the name.
    // For a namespaced call (smelt.ref), we'd have already handled it above,
    // so here we only deal with simple IDENT calls.
    let mut replaced = false;
    for child in node.children_with_tokens() {
        match child {
            SyntaxElement::Token(token) => {
                if !replaced && token.kind() == SyntaxKind::IDENT {
                    out.push_str(new_name);
                    replaced = true;
                } else {
                    out.push_str(token.text());
                }
            }
            SyntaxElement::Node(child_node) => {
                print_node(&child_node, ctx, out);
            }
        }
    }
}

/// Collect the JOIN clause text from a JOIN pipe stage.
///
/// The CST for a JOIN stage is:
///   PIPE_STAGE { PIPE_OP_JOIN (zero-width)  JOIN_CLAUSE { ... } }
///
/// We print the JOIN_CLAUSE node as SQL text and return it trimmed.
fn collect_join_clause_text(stage: &SyntaxNode, ctx: &PrintContext) -> String {
    use SyntaxKind::JOIN_CLAUSE;
    for child in stage.children() {
        if child.kind() == JOIN_CLAUSE {
            let mut text = String::new();
            print_node(&child, ctx, &mut text);
            return text.trim().to_string();
        }
    }
    // Fallback: collect everything after the PIPE_OP_JOIN marker.
    collect_stage_body_text(stage, ctx, SyntaxKind::PIPE_OP_JOIN)
}

/// Build a left-folded set-op expression from the prior (left) fragment and
/// the PIPE_STAGE containing the set-op operands.
///
/// `|> UNION ALL (q1), (q2)` with left `L` produces:
///   `L UNION ALL (q1) UNION ALL (q2)`
///
/// Implements Lowering rule 6.
fn build_set_op_expression(left: &str, stage: &SyntaxNode, ctx: &PrintContext) -> String {
    use SyntaxKind::*;

    // Determine operator keyword from the zero-width marker node kind.
    let op_marker_kind = stage.children().find_map(|c| {
        let k = c.kind();
        if matches!(k, PIPE_OP_UNION | PIPE_OP_INTERSECT | PIPE_OP_EXCEPT) {
            Some(k)
        } else {
            None
        }
    });
    let op_kw = match op_marker_kind {
        Some(PIPE_OP_INTERSECT) => "INTERSECT",
        Some(PIPE_OP_EXCEPT) => "EXCEPT",
        _ => "UNION",
    };

    // Determine modifier (ALL / DISTINCT) by scanning tokens after the op keyword token.
    let mut modifier = "";
    let mut found_op_kw = false;
    for elem in stage.children_with_tokens() {
        if let SyntaxElement::Token(t) = &elem {
            match t.kind() {
                UNION_KW | INTERSECT_KW | EXCEPT_KW => found_op_kw = true,
                ALL_KW if found_op_kw => {
                    modifier = " ALL";
                    break;
                }
                DISTINCT_KW if found_op_kw => {
                    modifier = " DISTINCT";
                    break;
                }
                LPAREN if found_op_kw => break,
                _ => {}
            }
        }
    }

    // Left-fold: start with `left`, then for each operand `(q)` append `OP MOD (q)`.
    let mut result = left.to_string();
    for child in stage.children() {
        let kind = child.kind();
        if kind == SUBQUERY || kind == PIPE_QUERY {
            let mut text = String::new();
            print_node(&child, ctx, &mut text);
            let trimmed = text.trim();
            result = format!("{result} {op_kw}{modifier} ({trimmed})");
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use smelt_parser::parse;

    fn duckdb_ctx() -> (SqlDialect, BackendCapabilities) {
        let caps = BackendCapabilities::duckdb();
        (caps.dialect, caps)
    }

    fn postgresql_ctx() -> (SqlDialect, BackendCapabilities) {
        let caps = BackendCapabilities::postgresql();
        (caps.dialect, caps)
    }

    fn spark_ctx() -> (SqlDialect, BackendCapabilities) {
        let caps = BackendCapabilities::spark();
        (caps.dialect, caps)
    }

    fn print_with(
        sql: &str,
        dialect: &SqlDialect,
        caps: &BackendCapabilities,
        schema: &str,
    ) -> String {
        let parsed = parse(sql);
        let ctx = PrintContext {
            dialect,
            capabilities: caps,
            schema,
            ephemeral_models: HashSet::new(),
            cross_engine_refs: HashMap::new(),
            smelt_as_struct: None,
            smelt_fn: None,
            smelt_path_ref: None,
            smelt_path_call: None,
        };
        print(&parsed.syntax(), &ctx)
    }

    // ===== Identity tests =====

    #[test]
    fn test_identity_simple_select() {
        let sql = "SELECT * FROM users";
        let (d, c) = duckdb_ctx();
        assert_eq!(print_with(sql, &d, &c, "main"), sql);
    }

    #[test]
    fn test_identity_complex_query() {
        let sql = "SELECT u.id, COUNT(*) AS cnt\nFROM users u\nWHERE u.active = 1\nGROUP BY u.id\nHAVING COUNT(*) > 5\nORDER BY cnt DESC\nLIMIT 10";
        let (d, c) = duckdb_ctx();
        assert_eq!(print_with(sql, &d, &c, "main"), sql);
    }

    #[test]
    fn test_identity_with_comments() {
        let sql = "-- This is a comment\nSELECT * FROM users";
        let (d, c) = duckdb_ctx();
        assert_eq!(print_with(sql, &d, &c, "main"), sql);
    }

    #[test]
    fn test_identity_with_cte() {
        let sql = "WITH active AS (SELECT * FROM users WHERE active = 1) SELECT * FROM active";
        let (d, c) = duckdb_ctx();
        assert_eq!(print_with(sql, &d, &c, "main"), sql);
    }

    #[test]
    fn test_identity_preserves_whitespace() {
        let sql =
            "SELECT\n    user_id,\n    COUNT(*) AS count\nFROM events\nWHERE event_type = 'click'";
        let (d, c) = duckdb_ctx();
        assert_eq!(print_with(sql, &d, &c, "main"), sql);
    }

    #[test]
    fn test_identity_preserves_doubled_quote_in_string_literal() {
        // SQL standard '' escape inside a single-quoted string must round-trip
        // verbatim so DuckDB sees the same literal it was given.
        let sql = "SELECT CASE WHEN x > 0 THEN 'Can''t Lose Them' ELSE 'Other' END AS label FROM t";
        let (d, c) = duckdb_ctx();
        assert_eq!(print_with(sql, &d, &c, "main"), sql);
    }

    // ===== Ref resolution tests =====

    #[test]
    fn test_ref_resolution() {
        let sql = "SELECT * FROM smelt.models.users";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT * FROM main.users");
    }

    #[test]
    fn test_ref_resolution_custom_schema() {
        let sql = "SELECT * FROM smelt.models.users";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "analytics");
        assert_eq!(result, "SELECT * FROM analytics.users");
    }

    #[test]
    fn test_multiple_refs() {
        let sql = "SELECT a.id, b.id FROM smelt.models.model_a a JOIN smelt.models.model_b b ON a.id = b.id";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert!(result.contains("main.model_a"));
        assert!(result.contains("main.model_b"));
        assert!(!result.contains("smelt.ref"));
    }

    // ===== Cross-engine ref resolution tests =====

    #[test]
    fn test_cross_engine_ref_resolution() {
        let sql = "SELECT * FROM smelt.models.spark_model";
        let parsed = parse(sql);
        let (d, c) = duckdb_ctx();
        let mut cross_refs = HashMap::new();
        cross_refs.insert(
            "spark_model".to_string(),
            "read_parquet('/data/warehouse/default/spark_model/**/*.parquet', hive_partitioning=true)".to_string(),
        );
        let ctx = PrintContext {
            dialect: &d,
            capabilities: &c,
            schema: "main",
            ephemeral_models: HashSet::new(),
            cross_engine_refs: cross_refs,
            smelt_as_struct: None,
            smelt_fn: None,
            smelt_path_ref: None,
            smelt_path_call: None,
        };
        let result = print(&parsed.syntax(), &ctx);
        assert!(
            result.contains("read_parquet("),
            "Expected read_parquet, got: {}",
            result
        );
        assert!(
            result.contains("spark_model/**/*.parquet"),
            "Expected parquet glob path, got: {}",
            result
        );
        assert!(
            !result.contains("main.spark_model"),
            "Should not contain schema-qualified ref, got: {}",
            result
        );
    }

    #[test]
    fn test_cross_engine_ref_mixed_with_normal_refs() {
        let sql = "SELECT a.id, b.id FROM smelt.models.local_model a JOIN smelt.models.spark_model b ON a.id = b.id";
        let parsed = parse(sql);
        let (d, c) = duckdb_ctx();
        let mut cross_refs = HashMap::new();
        cross_refs.insert(
            "spark_model".to_string(),
            "read_parquet('/data/spark_model/**/*.parquet', hive_partitioning=true)".to_string(),
        );
        let ctx = PrintContext {
            dialect: &d,
            capabilities: &c,
            schema: "main",
            ephemeral_models: HashSet::new(),
            cross_engine_refs: cross_refs,
            smelt_as_struct: None,
            smelt_fn: None,
            smelt_path_ref: None,
            smelt_path_call: None,
        };
        let result = print(&parsed.syntax(), &ctx);
        assert!(
            result.contains("main.local_model"),
            "Normal ref should resolve to schema.model, got: {}",
            result
        );
        assert!(
            result.contains("read_parquet("),
            "Cross-engine ref should resolve to read_parquet, got: {}",
            result
        );
    }

    // ===== Source resolution tests =====

    #[test]
    fn test_source_resolution() {
        let sql = "SELECT * FROM smelt.sources.raw.events";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT * FROM raw.events");
    }

    // ===== Formatting/whitespace preservation =====

    #[test]
    fn test_ref_preserves_surrounding_formatting() {
        let sql = "SELECT\n    user_id,\n    COUNT(*) as count\nFROM smelt.models.events\nWHERE event_type = 'click'";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert!(result.contains("SELECT\n    user_id,"));
        assert!(result.contains("FROM main.events"));
        assert!(result.contains("\nWHERE event_type = 'click'"));
    }

    // ===== QUALIFY rewrite tests =====

    #[test]
    fn test_qualify_rewrite_postgresql() {
        let sql = "SELECT *, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t QUALIFY rn = 1";
        let (d, c) = postgresql_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert!(
            result.contains("SELECT * FROM ("),
            "Expected subquery wrapper, got: {}",
            result
        );
        assert!(
            result.contains("WHERE rn = 1"),
            "Expected WHERE clause, got: {}",
            result
        );
        assert!(
            !result.contains("QUALIFY"),
            "QUALIFY should be removed, got: {}",
            result
        );
    }

    #[test]
    fn test_qualify_no_rewrite_duckdb() {
        let sql = "SELECT *, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t QUALIFY rn = 1";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert!(result.contains("QUALIFY"), "DuckDB should preserve QUALIFY");
        assert_eq!(result, sql);
    }

    // ===== ARRAY literal rewrite tests =====

    #[test]
    fn test_array_rewrite_spark() {
        let sql = "SELECT ARRAY[1, 2, 3] FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert!(
            result.contains("ARRAY(1, 2, 3)"),
            "Expected ARRAY() syntax, got: {}",
            result
        );
        assert!(
            !result.contains('['),
            "Brackets should be replaced, got: {}",
            result
        );
    }

    #[test]
    fn test_array_no_rewrite_duckdb() {
        let sql = "SELECT ARRAY[1, 2, 3] FROM t";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    // ===== DATE literal rewrite tests =====

    #[test]
    fn test_date_rewrite_spark() {
        let sql = "SELECT * FROM t WHERE d = DATE '2024-01-01'";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert!(
            result.contains("DATE('2024-01-01')"),
            "Expected DATE() function syntax, got: {}",
            result
        );
    }

    #[test]
    fn test_date_no_rewrite_duckdb() {
        let sql = "SELECT * FROM t WHERE d = DATE '2024-01-01'";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    // ===== :: cast rewrite tests =====

    #[test]
    fn test_double_colon_rewrite_spark() {
        let sql = "SELECT x::INTEGER FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT CAST(x AS INTEGER) FROM t");
    }

    #[test]
    fn test_double_colon_no_rewrite_duckdb() {
        let sql = "SELECT x::INTEGER FROM t";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    #[test]
    fn test_cast_function_passthrough_spark() {
        let sql = "SELECT CAST(x AS INTEGER) FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    #[test]
    fn test_double_colon_varchar_rewrite_spark() {
        let sql = "SELECT name::VARCHAR FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT CAST(name AS VARCHAR) FROM t");
    }

    // ===== Trailing comma removal tests =====

    #[test]
    fn test_trailing_comma_stripped_spark() {
        let sql = "SELECT a, b, FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        // The comma is removed; whitespace around it is preserved
        assert!(!result.contains("b,"), "Trailing comma should be removed");
        assert!(result.contains("a, b"), "Non-trailing commas preserved");
        assert!(!result.contains(", FROM"), "Comma before FROM removed");
    }

    #[test]
    fn test_trailing_comma_preserved_duckdb() {
        let sql = "SELECT a, b, FROM t";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    #[test]
    fn test_group_by_trailing_comma_stripped_spark() {
        let sql = "SELECT a, COUNT(*) FROM t GROUP BY a,";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT a, COUNT(*) FROM t GROUP BY a");
    }

    #[test]
    fn test_no_trailing_comma_unchanged_spark() {
        let sql = "SELECT a, b FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    // ===== EXPLODE/UNNEST renaming tests =====

    #[test]
    fn test_explode_to_unnest_duckdb() {
        let sql = "SELECT EXPLODE(arr) FROM t";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT UNNEST(arr) FROM t");
    }

    #[test]
    fn test_unnest_to_explode_spark() {
        let sql = "SELECT UNNEST(arr) FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT EXPLODE(arr) FROM t");
    }

    #[test]
    fn test_explode_unchanged_spark() {
        let sql = "SELECT EXPLODE(arr) FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    #[test]
    fn test_unnest_unchanged_duckdb() {
        let sql = "SELECT UNNEST(arr) FROM t";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    #[test]
    fn test_explode_to_unnest_postgresql() {
        let sql = "SELECT EXPLODE(arr) FROM t";
        let (d, c) = postgresql_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT UNNEST(arr) FROM t");
    }

    // ===== EVERY/BOOL_AND/BOOL_OR remapping tests =====

    #[test]
    fn test_every_to_bool_and_duckdb() {
        let sql = "SELECT EVERY(b) FROM t";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT BOOL_AND(b) FROM t");
    }

    #[test]
    fn test_every_unchanged_postgresql() {
        // PostgreSQL natively supports EVERY — no remapping needed
        let sql = "SELECT EVERY(b) FROM t";
        let (d, c) = postgresql_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    #[test]
    fn test_bool_and_to_every_spark() {
        let sql = "SELECT BOOL_AND(b) FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT EVERY(b) FROM t");
    }

    #[test]
    fn test_bool_or_to_some_spark() {
        let sql = "SELECT BOOL_OR(b) FROM t";
        let (d, c) = spark_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, "SELECT SOME(b) FROM t");
    }

    #[test]
    fn test_bool_and_unchanged_duckdb() {
        let sql = "SELECT BOOL_AND(b) FROM t";
        let (d, c) = duckdb_ctx();
        let result = print_with(sql, &d, &c, "main");
        assert_eq!(result, sql);
    }

    // ===== struct-returning call .* projection tests =====

    #[test]
    fn struct_returning_call_dot_star_lowers_to_field_projections() {
        let sql = "SELECT smelt.functions.parse_event_payload(payload).* FROM e";
        let parsed = parse(sql);
        let (d, c) = duckdb_ctx();
        // The expander returns a brace-struct literal body with three fields.
        let body =
            "{json_extract_string(payload, '$.event_name') AS event_name, json_extract_string(payload, '$.platform') AS platform, json_extract_string(payload, '$.url') AS url}";
        let ctx = PrintContext {
            dialect: &d,
            capabilities: &c,
            schema: "main",
            ephemeral_models: HashSet::new(),
            cross_engine_refs: HashMap::new(),
            smelt_as_struct: None,
            smelt_fn: None,
            smelt_path_ref: None,
            smelt_path_call: Some(Box::new(move |_segs, _pos, _named| Some(body.to_string()))),
        };
        let result = print(&parsed.syntax(), &ctx);
        // Should expand to three separate aliased projections, not the struct literal
        assert!(
            result.contains("json_extract_string(payload, '$.event_name') AS event_name"),
            "expected event_name projection, got: {result}"
        );
        assert!(
            result.contains("json_extract_string(payload, '$.platform') AS platform"),
            "expected platform projection, got: {result}"
        );
        assert!(
            result.contains("json_extract_string(payload, '$.url') AS url"),
            "expected url projection, got: {result}"
        );
        // Should NOT contain the brace-struct literal syntax (curly braces in the output)
        assert!(
            !result.contains('{'),
            "output should not contain struct literal braces, got: {result}"
        );
        assert!(
            !result.contains(".*"),
            "output should not contain .*, got: {result}"
        );
    }

    // ===== FROM-position derived-table alias synthesis tests =====

    /// Parse `SELECT * FROM smelt.functions.sessionize(x)` (no explicit alias).
    /// The printer must synthesise a `__smelt_t<N>` alias so DuckDB accepts the
    /// derived table.  The expanded body is a SELECT statement.
    #[test]
    fn table_expr_call_in_from_without_alias_synthesises_one() {
        let sql = "SELECT * FROM smelt.functions.sessionize(x)";
        let parsed = parse(sql);
        let (d, c) = duckdb_ctx();
        let body = "SELECT * FROM some_table WHERE x = 1";
        let ctx = PrintContext {
            dialect: &d,
            capabilities: &c,
            schema: "main",
            ephemeral_models: HashSet::new(),
            cross_engine_refs: HashMap::new(),
            smelt_as_struct: None,
            smelt_fn: None,
            smelt_path_ref: None,
            smelt_path_call: Some(Box::new(move |_segs, _pos, _named| Some(body.to_string()))),
        };
        let result = print(&parsed.syntax(), &ctx);
        // Must contain a synthesised alias beginning with __smelt_t
        assert!(
            result.contains("__smelt_t"),
            "expected synthesised __smelt_t alias, got: {result}"
        );
        // Expanded body must appear inside parentheses
        assert!(
            result.contains("(SELECT * FROM some_table WHERE x = 1)"),
            "expected expanded body in parens, got: {result}"
        );
        // The alias must follow the closing paren
        assert!(
            result.contains(") AS __smelt_t"),
            "expected ) AS __smelt_t<N>, got: {result}"
        );
    }

    /// Same input but with `AS s` after the call.  The printer must use the
    /// user's alias verbatim and must not emit a synthesised `__smelt_t` alias.
    #[test]
    fn table_expr_call_in_from_with_alias_passes_through() {
        let sql = "SELECT * FROM smelt.functions.sessionize(x) AS s";
        let parsed = parse(sql);
        let (d, c) = duckdb_ctx();
        let body = "SELECT * FROM some_table WHERE x = 1";
        let ctx = PrintContext {
            dialect: &d,
            capabilities: &c,
            schema: "main",
            ephemeral_models: HashSet::new(),
            cross_engine_refs: HashMap::new(),
            smelt_as_struct: None,
            smelt_fn: None,
            smelt_path_ref: None,
            smelt_path_call: Some(Box::new(move |_segs, _pos, _named| Some(body.to_string()))),
        };
        let result = print(&parsed.syntax(), &ctx);
        // Must NOT synthesise a __smelt_t alias
        assert!(
            !result.contains("__smelt_t"),
            "should not synthesise alias when user supplied one, got: {result}"
        );
        // The expanded body must still appear
        assert!(
            result.contains("SELECT * FROM some_table WHERE x = 1"),
            "expected expanded body in output, got: {result}"
        );
        // The user-supplied alias must appear
        assert!(
            result.contains("AS s"),
            "expected user alias AS s, got: {result}"
        );
    }

    /// A `smelt.<path>(args)` call in SELECT-list position (not FROM) must NOT
    /// get wrapped in `(...) AS __smelt_t<N>`.  The printer should expand the
    /// body verbatim (or the position is structurally unreachable for
    /// TableExpr-returning calls — either way no alias synthesis occurs).
    #[test]
    fn table_expr_call_in_select_list_does_not_synthesise_alias() {
        // In a SELECT list the path call is inside EXPRESSION/SELECT_ITEM, not TABLE_REF.
        // Even if the expander returns a SELECT body, no alias wrapping should occur.
        let sql = "SELECT smelt.functions.some_fn(x) FROM t";
        let parsed = parse(sql);
        let (d, c) = duckdb_ctx();
        let body = "some_expression";
        let ctx = PrintContext {
            dialect: &d,
            capabilities: &c,
            schema: "main",
            ephemeral_models: HashSet::new(),
            cross_engine_refs: HashMap::new(),
            smelt_as_struct: None,
            smelt_fn: None,
            smelt_path_ref: None,
            smelt_path_call: Some(Box::new(move |_segs, _pos, _named| Some(body.to_string()))),
        };
        let result = print(&parsed.syntax(), &ctx);
        // Must NOT synthesise a __smelt_t alias in SELECT list position
        assert!(
            !result.contains("__smelt_t"),
            "should not synthesise alias in SELECT list position, got: {result}"
        );
        // The expanded body must appear verbatim
        assert!(
            result.contains("some_expression"),
            "expected expanded body, got: {result}"
        );
    }

    // ===== Nested smelt.define fixpoint tests (BUG-013) =====

    /// Printing a model SQL that calls `smelt.functions.outer(x)` where `outer`
    /// expands to `(smelt.functions.inner(x))` and `inner` expands to `(x + 1)`
    /// must produce output containing `(x + 1)` (not `smelt.functions.inner`).
    ///
    /// Before the fix, the reparse step did not produce `SMELT_PATH_CALL` nodes
    /// from a bare/parenthesised fragment, so the inner path-call was passed
    /// through verbatim to DuckDB → `Catalog "smelt" does not exist`.
    #[test]
    fn nested_define_chain_expands_to_fixpoint() {
        // Two-level chain: wrap_fn → (smelt.functions.increment(x)), increment → (x + 1).
        // "wrap_fn" and "increment" are plain identifiers with no SQL keyword
        // collision, so the parser reliably recognises the call as SMELT_PATH_CALL.
        let sql = "SELECT smelt.functions.wrap_fn(x) FROM t";
        let parsed = parse(sql);
        let (d, c) = duckdb_ctx();

        let wrap_body = "(smelt.functions.increment(x))";
        let increment_body = "(x + 1)";
        let ctx = PrintContext {
            dialect: &d,
            capabilities: &c,
            schema: "main",
            ephemeral_models: HashSet::new(),
            cross_engine_refs: HashMap::new(),
            smelt_as_struct: None,
            smelt_fn: None,
            smelt_path_ref: None,
            smelt_path_call: Some(Box::new(move |segs, _pos, _named| {
                match segs.last().map(|s| s.as_str()) {
                    Some("wrap_fn") => Some(wrap_body.to_string()),
                    Some("increment") => Some(increment_body.to_string()),
                    _ => None,
                }
            })),
        };
        let result = print(&parsed.syntax(), &ctx);

        // The final output must contain the fully expanded arithmetic, not any
        // residual smelt.functions.* reference.
        assert!(
            !result.contains("smelt.functions"),
            "nested smelt.functions.* must be fully expanded, got: {result}"
        );
        assert!(
            result.contains("x + 1"),
            "expected fully expanded body '(x + 1)', got: {result}"
        );
    }

    /// BUG-018: a block `PASSING <name> AS (<body>)` clause must arrive at the
    /// expander as a named binding (`<name>` → `<body>`), so a fragment
    /// parameter supplied via PASSING is substituted rather than falling back
    /// to its default.
    #[test]
    fn block_passing_binds_fragment_into_named_args() {
        // `metrics` is supplied via a trailing PASSING block, not inline.
        let sql = "SELECT * FROM smelt.functions.rollup(t) PASSING metrics AS (COUNT(*))";
        let parsed = parse(sql);
        let (d, c) = duckdb_ctx();

        let ctx = PrintContext {
            dialect: &d,
            capabilities: &c,
            schema: "main",
            ephemeral_models: HashSet::new(),
            cross_engine_refs: HashMap::new(),
            smelt_as_struct: None,
            smelt_fn: None,
            smelt_path_ref: None,
            // The expander mimics body substitution: it reads the `metrics`
            // binding out of `named` and splices it into the body. If the
            // PASSING clause were ignored, `metrics` would fall back to `()`.
            smelt_path_call: Some(Box::new(move |_segs, _pos, named| {
                let metrics = named
                    .iter()
                    .find(|(k, _)| k == "metrics")
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| "()".to_string());
                Some(format!(
                    "(SELECT group_col, {metrics} FROM t GROUP BY group_col)"
                ))
            })),
        };
        let result = print(&parsed.syntax(), &ctx);

        assert!(
            result.contains("COUNT(*)"),
            "PASSING body must be bound to `metrics`, got: {result}"
        );
        assert!(
            !result.contains("group_col, ()"),
            "`metrics` must not fall back to its default `()`, got: {result}"
        );
    }
}
