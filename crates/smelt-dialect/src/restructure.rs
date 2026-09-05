//! Statement-level restructure planning.
//!
//! Some built-ins cannot be lowered by substituting one expression for
//! another, because the target backend offers the operation only in the
//! *other* call position from the one the author wrote. This module is the
//! pure planner: given a query block and a dialect, it returns either a
//! [`RestructurePlan`] — data describing how to rewrite the block around a
//! synthesised CTE — or the refusals the admissibility rules require.
//! Nothing here prints SQL; a later stage consumes the plan.
//!
//! Correctness oracle: `docs/specs/multi_backend.md` §"Statement-level
//! lowering". The admissibility rules and the refusal of running-frame
//! windows are settled decisions, argued out against a live warehouse —
//! this module enforces them, it does not relitigate them.

use smelt_parser::ast::{
    Expr, FunctionCall, GroupByClause, OrderByClause, SelectStmt, SortDirection,
};
use smelt_parser::syntax_kind::{SyntaxKind, SyntaxNode};
use smelt_parser::{TextRange, TextSize};
use smelt_types::signatures::RestructureId;
use smelt_types::{BuiltinRegistry, DialectId, Emission};

use crate::position::classify as classify_position;
use crate::SqlDialect;
use crate::UnsupportedEmission;

/// Built-in names with no stable value across two evaluations, or across the
/// two evaluations a repeated `FROM` would otherwise cause. Mirrors the
/// nondeterministic set the cross-engine audit already treats as
/// `SchemaOnly` (`docs/specs/multi_backend.md` §"Cross-engine emission
/// audit"): `RANDOM`, `NOW`, `CURRENT_DATE`, `UUID`. A `PARTITION BY`
/// expression that calls one of these is refused rather than bound once and
/// silently made deterministic, or evaluated twice and silently made
/// inconsistent.
const NONDETERMINISTIC_BUILTINS: &[&str] = &[
    "RANDOM",
    "NOW",
    "CURRENT_DATE",
    "CURRENT_TIMESTAMP",
    "CURRENT_TIME",
    "UUID",
];

// ─── Plan data ──────────────────────────────────────────────────────────────

/// The source every synthesised CTE in a plan reads from: the query block's
/// own `FROM`, and — only for [`RestructurePlan::WindowToCte`] — its `WHERE`,
/// planted inside the bound source rather than on the join. Window functions
/// are evaluated after `WHERE`, so a predicate left outside would let a
/// grouped CTE aggregate rows the original query had already discarded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundSource {
    /// The synthesised alias: `__smelt_base` (`WindowToCte`) or `__smelt_r0`
    /// (`AnalyticToCte`).
    pub alias: String,
    /// The original `FROM_CLAUSE` node, unchanged.
    pub from: SyntaxNode,
    /// The original `WHERE` predicate, if any — planted inside the bound
    /// source. `None` for `AnalyticToCte`, whose single CTE already carries
    /// the block's `WHERE` as its own filter rather than a joined-back one.
    pub where_predicate: Option<SyntaxNode>,
}

/// One call rewritten to read a grouped CTE's value column back through a
/// null-safe join.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowCallReplacement {
    /// The original `FUNCTION_CALL` node in the source CST.
    pub call: SyntaxNode,
    /// The column the grouped CTE computes for this call.
    pub value_column: String,
    /// The call's own `OVER` clause, if it has one — parsed as a
    /// wrapper-level sibling of the `FUNCTION_CALL` node rather than a child
    /// of it. Resolved once here, at plan time, so the printer never
    /// re-derives a call's structural surroundings itself
    /// (`docs/specs/multi_backend.md` §"Emission is scoped to call
    /// position"): it substitutes this node (with an empty replacement,
    /// swallowing it) wherever the call is nested inside a select item's
    /// expression, exactly like the call itself.
    pub over_clause: Option<SyntaxNode>,
}

/// One grouped CTE — the source bound once, grouped by `partition_keys`,
/// serving every call in `calls` that shares those exact keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupBinding {
    /// The synthesised CTE name: `__smelt_w0`, `__smelt_w1`, ….
    pub cte_name: String,
    /// The partition-key expressions, source CST nodes, unchanged. Empty
    /// means the calls had no `PARTITION BY` — a one-row CTE, joined with a
    /// `CROSS JOIN`.
    pub partition_keys: Vec<SyntaxNode>,
    pub calls: Vec<WindowCallReplacement>,
}

/// One call rewritten to read an analytic column back through `ANY_VALUE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalyticCallReplacement {
    /// The original `FUNCTION_CALL` node in the source CST (the ordered-set
    /// aggregate with its `WITHIN GROUP` clause).
    pub call: SyntaxNode,
    /// The analytic column the bound source computes for this call.
    pub value_column: String,
    /// `true` when the `WITHIN GROUP (ORDER BY … DESC)` sort key was
    /// descending — the analytic form's fraction argument is `1 - f` rather
    /// than `f` (`docs/specs/multi_backend.md` §"Statement-level lowering":
    /// "A `DESC` sort key inverts the fraction").
    pub fraction_complement: bool,
}

/// A pure, printer-agnostic description of a statement-level restructure for
/// one query block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestructurePlan {
    /// An aggregate-only built-in reached with a whole-partition `OVER`
    /// clause: the source is bound once, grouped by each affected call's
    /// partition keys, and joined back. Admissible only at
    /// `Position::WholePartitionWindow` — a running window has no correct
    /// CTE form.
    WindowToCte {
        select_stmt: SyntaxNode,
        base: BoundSource,
        groups: Vec<GroupBinding>,
    },
    /// An analytic-only built-in reached under `GROUP BY`: the query's
    /// `FROM`/`WHERE` move into a CTE that adds the value as an analytic
    /// column over the grouping keys, read back through `ANY_VALUE`.
    AnalyticToCte {
        select_stmt: SyntaxNode,
        source: BoundSource,
        /// The block's own `GROUP BY` key expressions — planted as the
        /// analytic column's `PARTITION BY` inside the bound source.
        group_keys: Vec<SyntaxNode>,
        replacements: Vec<AnalyticCallReplacement>,
    },
}

// ─── Entry point ────────────────────────────────────────────────────────────

/// Plan every statement-level restructure `root` needs on `dialect`.
///
/// Walks `root` for calls the registry declares [`Emission::Restructure`] at
/// their classified position, groups them by their enclosing query block
/// (`SELECT_STMT`), and plans each block independently. A call the registry
/// declares [`Emission::Unsupported`] at its position (the running-window
/// refusal, stated as an ordinary verdict — nothing about it is
/// special-cased) is reported the same way admissibility refusals are.
///
/// Pure: no I/O, no printing. `Ok(&[])` means nothing on `root` needs
/// restructuring on `dialect`.
pub fn plan(
    root: &SyntaxNode,
    dialect: SqlDialect,
) -> Result<Vec<RestructurePlan>, Vec<UnsupportedEmission>> {
    let id = dialect.id();
    let mut refusals: Vec<UnsupportedEmission> = Vec::new();
    let mut candidates: Vec<(SyntaxNode, RestructureId)> = Vec::new();

    for node in root.descendants() {
        if node.kind() != SyntaxKind::FUNCTION_CALL {
            continue;
        }
        let Some(name) = FunctionCall::cast(node.clone()).and_then(|fc| fc.name()) else {
            continue;
        };
        let Some(sig) = BuiltinRegistry::resolve(&name) else {
            continue;
        };
        let position = classify_position(&node, root);
        match sig.emission_at(id, position) {
            Emission::Restructure(rid) => candidates.push((node.clone(), rid)),
            Emission::Unsupported { reason } => refusals.push(UnsupportedEmission {
                name: sig.name.as_str(),
                dialect: id,
                reason,
                range: trimmed_range(&node),
            }),
            Emission::Native
            | Emission::Rename(_)
            | Emission::Rewrite(_)
            | Emission::Template(_) => {}
        }
    }

    // Group candidates by their nearest enclosing SELECT_STMT — the query
    // block a restructure is scoped to (`docs/specs/multi_backend.md`
    // §"Statement-level lowering": "The restructure applies to one query
    // block").
    let mut blocks: Vec<(SyntaxNode, Vec<(SyntaxNode, RestructureId)>)> = Vec::new();
    for (call, rid) in candidates {
        let Some(block) = call
            .ancestors()
            .find(|n| n.kind() == SyntaxKind::SELECT_STMT)
        else {
            continue;
        };
        match blocks.iter_mut().find(|(b, _)| *b == block) {
            Some((_, v)) => v.push((call, rid)),
            None => blocks.push((block, vec![(call, rid)])),
        }
    }

    let mut plans = Vec::new();
    for (block, calls) in blocks {
        match plan_block(&block, id, &calls) {
            Ok(p) => plans.push(p),
            Err(mut e) => refusals.append(&mut e),
        }
    }

    if !refusals.is_empty() {
        return Err(refusals);
    }
    Ok(plans)
}

// ─── Per-block planning ─────────────────────────────────────────────────────

fn plan_block(
    block: &SyntaxNode,
    dialect: DialectId,
    calls: &[(SyntaxNode, RestructureId)],
) -> Result<RestructurePlan, Vec<UnsupportedEmission>> {
    let select = SelectStmt::cast(block.clone()).ok_or_else(|| {
        vec![refusal(
            &calls[0].0,
            dialect,
            "the affected call's enclosing query block could not be read as a SELECT statement",
        )]
    })?;

    // All calls in one block must ask for the same restructure shape — a
    // block mixing both is not a shape this planner takes a position on.
    let direction = calls[0].1;
    if calls.iter().any(|(_, d)| *d != direction) {
        return Err(calls
            .iter()
            .map(|(call, _)| {
                refusal(
                    call,
                    dialect,
                    "this query block needs two different statement-level restructures at \
                     once, which this planner does not support",
                )
            })
            .collect());
    }

    if let Some(reason) = correlated_block_reason(block) {
        return Err(calls
            .iter()
            .map(|(call, _)| refusal(call, dialect, reason))
            .collect());
    }

    let mut refusals = Vec::new();

    // Rule 1: plain GROUP BY only — ROLLUP/CUBE/GROUPING SETS compute
    // super-aggregate rows no PARTITION BY produces.
    if let Some(group_by) = select.group_by_clause() {
        if let Some(reason) = grouping_admissibility_reason(&group_by) {
            for (call, _) in calls {
                refusals.push(refusal(call, dialect, reason));
            }
        }
    }

    // Rule 2: every occurrence of the affected built-in is in the select
    // list. HAVING / (the block's own) ORDER BY / QUALIFY are refused.
    let having = block
        .children()
        .find(|n| n.kind() == SyntaxKind::HAVING_CLAUSE);
    let qualify = block
        .children()
        .find(|n| n.kind() == SyntaxKind::QUALIFY_CLAUSE);
    let order_by = select.order_by_clause();

    for (call, _) in calls {
        let fc = FunctionCall::cast(call.clone());
        let name = fc.as_ref().and_then(|fc| fc.name());
        let canonical = name
            .as_deref()
            .and_then(BuiltinRegistry::canonical_name)
            .unwrap_or("");

        if occurs_in(having.as_ref(), canonical)
            || occurs_in(order_by.as_ref().map(|o| o.syntax()), canonical)
            || occurs_in(qualify.as_ref(), canonical)
        {
            refusals.push(refusal(
                call,
                dialect,
                "this built-in also occurs outside the select list (HAVING, ORDER BY, or \
                 QUALIFY); the restructure would still leave the construct it exists to remove",
            ));
        }

        // Rule 3: no DISTINCT, no FILTER — neither has an analytic form.
        if let Some(fc) = &fc {
            if call_is_distinct(fc) {
                refusals.push(refusal(
                    call,
                    dialect,
                    "DISTINCT has no analytic form on any supported backend",
                ));
            }
            if fc.filter_clause().is_some() {
                refusals.push(refusal(
                    call,
                    dialect,
                    "FILTER (WHERE …) has no analytic form on any supported backend",
                ));
            }
        }
    }

    // Rule 4: no unexpanded wildcard — `SELECT *` would expand against the
    // restructured FROM and pick up the synthesised columns.
    if let Some(list) = select.select_list() {
        if list.items().any(|item| item.is_wildcard()) {
            for (call, _) in calls {
                refusals.push(refusal(
                    call,
                    dialect,
                    "the select list has an unexpanded wildcard, which would expand against \
                     the restructured FROM and pick up the synthesised columns",
                ));
            }
        }
    }

    if !refusals.is_empty() {
        return Err(refusals);
    }

    match direction {
        RestructureId::WindowToCte => plan_window_to_cte(block, &select, dialect, calls),
        RestructureId::AnalyticToCte => plan_analytic_to_cte(block, &select, dialect, calls),
    }
}

/// `None` when `block` sits somewhere a synthesised CTE can be planted in
/// place: the top-level statement, an author-written CTE body, or a
/// `FROM`-clause derived table. Anything else — a scalar subquery, an
/// `IN`/`EXISTS` subquery, a bare `UNION` operand — is correlated-shaped or
/// otherwise not self-contained, and refusing is the safe direction: a
/// correlated subquery whose block would need a hoisted CTE has no correct
/// local rewrite.
fn correlated_block_reason(block: &SyntaxNode) -> Option<&'static str> {
    let Some(parent) = block.parent() else {
        return None; // the block is the whole tree — nothing above it
    };
    if parent.kind() == SyntaxKind::FILE {
        return None;
    }
    if parent.kind() == SyntaxKind::SUBQUERY {
        return match parent.parent() {
            Some(gp) if gp.kind() == SyntaxKind::CTE => None,
            Some(gp) if gp.kind() == SyntaxKind::TABLE_REF => None,
            _ => Some(
                "this call sits inside a correlated subquery; hoisting a CTE for it would not \
                 be self-contained",
            ),
        };
    }
    Some(
        "this call's enclosing query block is not a top-level statement, an author-written \
         CTE, or a FROM-clause subquery, so it cannot host a synthesised CTE in place",
    )
}

/// The admissibility reason a `GROUP BY` is not plain, if any: `ROLLUP`,
/// `CUBE`, or `GROUPING SETS`.
fn grouping_admissibility_reason(group_by: &GroupByClause) -> Option<&'static str> {
    let has_grouping_sets = group_by
        .syntax()
        .children()
        .any(|n| n.kind() == SyntaxKind::GROUPING_SETS_CLAUSE);
    if has_grouping_sets {
        return Some(
            "GROUP BY GROUPING SETS computes super-aggregate rows that no PARTITION BY \
             produces",
        );
    }
    for expr in group_by.expressions() {
        if let Some(fc) = expr.as_function_call() {
            if let Some(name) = fc.name() {
                let upper = name.to_ascii_uppercase();
                if upper == "ROLLUP" {
                    return Some(
                        "GROUP BY ROLLUP computes super-aggregate rows that no PARTITION BY \
                         produces",
                    );
                }
                if upper == "CUBE" {
                    return Some(
                        "GROUP BY CUBE computes super-aggregate rows that no PARTITION BY \
                         produces",
                    );
                }
            }
        }
    }
    None
}

/// Whether `scope`'s subtree contains a `FUNCTION_CALL` whose canonical
/// registry name matches `canonical` — used to check an affected built-in's
/// occurrence outside the select list. `canonical` is compared by the
/// registry's own canonical spelling so an alias (`MAX_BY` vs `ARG_MAX`)
/// still matches.
fn occurs_in(scope: Option<&SyntaxNode>, canonical: &str) -> bool {
    let Some(scope) = scope else { return false };
    if canonical.is_empty() {
        return false;
    }
    scope
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
        .filter_map(|n| FunctionCall::cast(n).and_then(|fc| fc.name()))
        .any(|name| BuiltinRegistry::canonical_name(&name) == Some(canonical))
}

/// Whether `call`'s own argument list carries a `DISTINCT` modifier
/// (`COUNT(DISTINCT x)`). The parser emits `DISTINCT_KW` as a direct token
/// child of `ARG_LIST` (`parser/smelt_ext.rs::parse_argument`); there is no
/// dedicated AST accessor for it.
fn call_is_distinct(fc: &FunctionCall) -> bool {
    fc.syntax()
        .children()
        .find(|n| n.kind() == SyntaxKind::ARG_LIST)
        .is_some_and(|arg_list| {
            arg_list
                .children_with_tokens()
                .filter_map(|e| e.into_token())
                .any(|t| t.kind() == SyntaxKind::DISTINCT_KW)
        })
}

/// The node's span with trailing trivia removed — matches
/// `emission_check::trimmed_range` so a restructure refusal underlines the
/// same span an ordinary emission refusal would.
fn trimmed_range(node: &SyntaxNode) -> TextRange {
    let range = node.text_range();
    let text = node.text().to_string();
    let trailing = text.len() - text.trim_end().len();
    TextRange::new(range.start(), range.end() - TextSize::from(trailing as u32))
}

fn refusal(call: &SyntaxNode, dialect: DialectId, reason: &'static str) -> UnsupportedEmission {
    let name = FunctionCall::cast(call.clone())
        .and_then(|fc| fc.name())
        .as_deref()
        .and_then(BuiltinRegistry::canonical_name)
        .unwrap_or("");
    UnsupportedEmission {
        name,
        dialect,
        reason,
        range: trimmed_range(call),
    }
}

// ─── WindowToCte ────────────────────────────────────────────────────────────

fn plan_window_to_cte(
    block: &SyntaxNode,
    select: &SelectStmt,
    dialect: DialectId,
    calls: &[(SyntaxNode, RestructureId)],
) -> Result<RestructurePlan, Vec<UnsupportedEmission>> {
    let Some(from) = block
        .children()
        .find(|n| n.kind() == SyntaxKind::FROM_CLAUSE)
    else {
        return Err(calls
            .iter()
            .map(|(call, _)| refusal(call, dialect, "the query block has no FROM clause"))
            .collect());
    };
    let where_predicate = select
        .where_clause()
        .and_then(|w| w.expression())
        .map(|e| e.syntax().clone());

    let base = BoundSource {
        alias: "__smelt_base".to_string(),
        from,
        where_predicate,
    };

    let mut refusals = Vec::new();
    let mut groups: Vec<GroupBinding> = Vec::new();

    for (call, _) in calls {
        let window_spec = window_spec_of(call);
        let partition_keys = window_spec
            .as_ref()
            .map(partition_keys_of)
            .unwrap_or_default();

        if let Some(reason) = nondeterministic_key_reason(&partition_keys) {
            refusals.push(refusal(call, dialect, reason));
            continue;
        }

        let group_idx = groups
            .iter()
            .position(|g| keys_match(&g.partition_keys, &partition_keys));
        let idx = match group_idx {
            Some(i) => i,
            None => {
                groups.push(GroupBinding {
                    cte_name: format!("__smelt_w{}", groups.len()),
                    partition_keys,
                    calls: Vec::new(),
                });
                groups.len() - 1
            }
        };
        let value_column = format!("v{}", groups[idx].calls.len());
        groups[idx].calls.push(WindowCallReplacement {
            call: call.clone(),
            value_column,
            over_clause: window_spec,
        });
    }

    if !refusals.is_empty() {
        return Err(refusals);
    }

    Ok(RestructurePlan::WindowToCte {
        select_stmt: block.clone(),
        base,
        groups,
    })
}

/// `call`'s own `OVER` clause, if it has one — parsed as a wrapper-level
/// sibling of the `FUNCTION_CALL` node rather than a child of it. The parser
/// emits `WINDOW_SPEC` (when present) as the node-sibling immediately
/// following its `FUNCTION_CALL`, so `call.next_sibling()` is *this call's*
/// `OVER` clause — never a later sibling call's. `None` when the call has no
/// `OVER` clause at all (the immediate next sibling, if any, is something
/// else — e.g. another `FUNCTION_CALL` beside it with no `OVER`).
fn window_spec_of(call: &SyntaxNode) -> Option<SyntaxNode> {
    let next = call.next_sibling()?;
    (next.kind() == SyntaxKind::WINDOW_SPEC).then_some(next)
}

/// The `PARTITION BY` key expressions of an `OVER` clause. Empty when there
/// is no `PARTITION BY` — the degenerate one-row-CTE case.
fn partition_keys_of(spec: &SyntaxNode) -> Vec<SyntaxNode> {
    let Some(partition_by) = spec
        .children()
        .find(|n| n.kind() == SyntaxKind::PARTITION_BY_CLAUSE)
    else {
        return Vec::new();
    };
    partition_by
        .children()
        .filter_map(Expr::cast)
        .map(|e| e.syntax().clone())
        .collect()
}

/// The refusal reason when any partition key expression calls a
/// nondeterministic built-in — the source is bound once, so a
/// nondeterministic key cannot be evaluated twice, and it cannot be bound
/// once and silently treated as stable either.
fn nondeterministic_key_reason(keys: &[SyntaxNode]) -> Option<&'static str> {
    let any_nondeterministic = keys.iter().any(|key| {
        key.descendants()
            .filter(|n| n.kind() == SyntaxKind::FUNCTION_CALL)
            .filter_map(|n| FunctionCall::cast(n).and_then(|fc| fc.name()))
            .any(|name| NONDETERMINISTIC_BUILTINS.contains(&name.to_ascii_uppercase().as_str()))
    });
    if any_nondeterministic {
        Some(
            "a non-deterministic PARTITION BY expression cannot be bound once and evaluated \
             consistently, or evaluated twice consistently",
        )
    } else {
        None
    }
}

/// Structural key-set equality, by source text — sufficient here because two
/// key sets are compared only within one already-parsed query block, never
/// across dialects or across a rewrite.
fn keys_match(a: &[SyntaxNode], b: &[SyntaxNode]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .all(|(x, y)| x.text().to_string() == y.text().to_string())
}

/// Read the `WITHIN GROUP (ORDER BY <sort_key> [DESC] [NULLS FIRST|LAST])`
/// clause off an ordered-set aggregate call, returning the sort key
/// expression and whether the analytic form's fraction argument must be
/// complemented (`1 - f`) because the sort was descending
/// (`docs/specs/multi_backend.md` §"Statement-level lowering": "A `DESC`
/// sort key inverts the fraction"). A `NULLS FIRST`/`LAST` modifier the
/// analytic form cannot express is refused rather than silently dropped.
///
/// Shared by `plan_analytic_to_cte` (a `GROUP BY` call restructured around an
/// analytic CTE) and the `WithinGroupToAnalytic` expression rewrite (a
/// whole-partition-window call rewritten to the analytic spelling in place)
/// — both read the same clause under the same admissibility rule.
pub(crate) fn within_group_sort_key(call: &SyntaxNode) -> Result<(SyntaxNode, bool), &'static str> {
    let within_group = call
        .children()
        .find(|n| n.kind() == SyntaxKind::WITHIN_GROUP_CLAUSE)
        .ok_or(
            "this analytic-only built-in has no WITHIN GROUP sort key to plant as its \
             analytic ORDER BY",
        )?;
    let order_by = within_group
        .children()
        .find(|n| n.kind() == SyntaxKind::ORDER_BY_CLAUSE)
        .and_then(OrderByClause::cast)
        .ok_or("this analytic-only built-in's WITHIN GROUP clause has no ORDER BY")?;
    let sort_item = order_by
        .items()
        .next()
        .ok_or("this analytic-only built-in's WITHIN GROUP ORDER BY has no sort key")?;

    // A NULLS FIRST/LAST modifier the analytic form cannot express is
    // refused, never dropped.
    if sort_item.null_ordering().is_some() {
        return Err(
            "a NULLS FIRST/LAST modifier on the WITHIN GROUP sort key cannot be expressed \
             by the analytic form and is refused rather than silently dropped",
        );
    }

    let fraction_complement = matches!(sort_item.direction(), Some(SortDirection::Desc));
    let sort_expr = sort_item
        .expression()
        .ok_or("this analytic-only built-in's WITHIN GROUP sort key has no expression")?;
    Ok((sort_expr.syntax().clone(), fraction_complement))
}

// ─── AnalyticToCte ──────────────────────────────────────────────────────────

fn plan_analytic_to_cte(
    block: &SyntaxNode,
    select: &SelectStmt,
    dialect: DialectId,
    calls: &[(SyntaxNode, RestructureId)],
) -> Result<RestructurePlan, Vec<UnsupportedEmission>> {
    let Some(from) = block
        .children()
        .find(|n| n.kind() == SyntaxKind::FROM_CLAUSE)
    else {
        return Err(calls
            .iter()
            .map(|(call, _)| refusal(call, dialect, "the query block has no FROM clause"))
            .collect());
    };
    let where_predicate = select
        .where_clause()
        .and_then(|w| w.expression())
        .map(|e| e.syntax().clone());

    // `AnalyticToCte`'s single CTE carries the block's own WHERE as its own
    // filter (see the worked example in the spec) — the same field the
    // `WindowToCte` shape uses for its bound source's filter.
    let source = BoundSource {
        alias: "__smelt_r0".to_string(),
        from,
        where_predicate,
    };

    let group_keys: Vec<SyntaxNode> = select
        .group_by_clause()
        .map(|g| g.expressions().map(|e| e.syntax().clone()).collect())
        .unwrap_or_default();

    let mut refusals = Vec::new();
    let mut replacements = Vec::new();

    for (call, _) in calls {
        let fraction_complement = match within_group_sort_key(call) {
            Ok((_sort_expr, fraction_complement)) => fraction_complement,
            Err(reason) => {
                refusals.push(refusal(call, dialect, reason));
                continue;
            }
        };

        replacements.push(AnalyticCallReplacement {
            call: call.clone(),
            value_column: format!("v{}", replacements.len()),
            fraction_complement,
        });
    }

    if !refusals.is_empty() {
        return Err(refusals);
    }

    Ok(RestructurePlan::AnalyticToCte {
        select_stmt: block.clone(),
        source,
        group_keys,
        replacements,
    })
}
