//! Pipe-syntax lowering: a `|>` stage chain rewritten into a plain
//! `SELECT` statement, driving the per-stage body collectors in
//! `pipe_stages`.

use smelt_parser::syntax_kind::{SyntaxElement, SyntaxKind, SyntaxNode};

use super::pipe_stages::{
    collect_aggregate_parts, collect_as_alias, collect_from_body, collect_join_clause_text,
    collect_limit_body, collect_order_by_body, collect_select_body, collect_stage_body_text,
    collect_where_body, parse_column_list, parse_rename_pairs, parse_set_assignments,
};
use super::print_node;
use super::rewrites::print_children;
use super::PrintContext;

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
pub(crate) fn print_pipe_rewrite(node: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
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
pub(crate) fn reexpand_call_body(expanded: &str, ctx: &PrintContext) -> String {
    let wrapped = format!("SELECT {expanded}");
    let reparsed = smelt_parser::parse(&wrapped);
    let mut out = String::new();
    print_node(&reparsed.syntax(), ctx, &mut out);
    out.strip_prefix("SELECT ")
        .expect("synthetic SELECT wrapper prefix is always present (print_node on a FILE starts with the SELECT keyword)")
        .to_string()
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
