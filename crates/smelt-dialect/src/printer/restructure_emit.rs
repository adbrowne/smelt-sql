//! Statement-level restructure emission: turning a planned
//! `RestructurePlan` into a synthesised-CTE `SELECT`, and the thread-local
//! substitution / position-override frames that carry it through the walk.

use smelt_parser::syntax_kind::{SyntaxKind, SyntaxNode};
use smelt_parser::FunctionCall;

use std::cell::RefCell;

use crate::restructure::{AnalyticCallReplacement, BoundSource, GroupBinding, RestructurePlan};
use crate::{BackendCapabilities, NullSafeEqualitySpelling};
use smelt_parser::ast::{OrderByClause, SelectEntry, SelectItem, SelectStmt};
use smelt_types::signatures::Position;
use smelt_types::{BuiltinRegistry, Emission};

use super::print_node;
use super::rewrites::print_children;
use super::PrintContext;

// ─── Statement-level restructure emission ──────────────────────────────────
//
// Turns a `RestructurePlan` (`crate::restructure`) into SQL: a synthesised
// CTE appended to the author's own `WITH` list, base references qualified to
// the bound source's alias, and — for `WindowToCte` — a null-safe join whose
// spelling comes from `BackendCapabilities::null_safe_equality`, never from a
// dialect arm. Correctness oracle: `docs/specs/multi_backend.md`
// §"Statement-level lowering".

/// The `SELECT_STMT` node a plan was computed for, read out of whichever
/// variant it is — used to match a plan to the node currently being printed.
pub(crate) fn restructure_plan_select_stmt(plan: &RestructurePlan) -> &SyntaxNode {
    match plan {
        RestructurePlan::WindowToCte { select_stmt, .. } => select_stmt,
        RestructurePlan::AnalyticToCte { select_stmt, .. } => select_stmt,
    }
}

pub(crate) fn print_restructured_select(
    plan: &RestructurePlan,
    ctx: &PrintContext,
    out: &mut String,
) {
    match plan {
        RestructurePlan::WindowToCte {
            select_stmt,
            base,
            groups,
        } => print_window_to_cte(select_stmt, base, groups, ctx, out),
        RestructurePlan::AnalyticToCte {
            select_stmt,
            source,
            group_keys,
            replacements,
        } => print_analytic_to_cte(select_stmt, source, group_keys, replacements, ctx, out),
    }
}

/// Print the author's own `WITH` items (if any), each followed by `", "` —
/// ready for a synthesised CTE to be appended right after
/// (`docs/specs/multi_backend.md`: "a synthesised CTE is appended to the
/// author's `WITH` list rather than prefixed to the statement").
fn print_author_with_items(block: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    let Some(with_clause) = block
        .children()
        .find(|n| n.kind() == SyntaxKind::WITH_CLAUSE)
    else {
        return;
    };
    for child in with_clause.children() {
        if child.kind() == SyntaxKind::CTE {
            out.push_str(&print_trimmed(&child, ctx));
            out.push_str(", ");
        }
    }
}

/// Print `node`'s SQL, trimmed. A small buffer-and-trim wrapper used
/// throughout restructure emission, where fragments are assembled from
/// several printed pieces rather than one pass over the tree.
pub(crate) fn print_trimmed(node: &SyntaxNode, ctx: &PrintContext) -> String {
    let mut s = String::new();
    print_node(node, ctx, &mut s);
    s.trim().to_string()
}

/// Whether `text` is a bare, unqualified identifier — the only shape this
/// printer qualifies to a bound source's alias. A qualified reference
/// (contains `.`) or a compound expression (contains `(`) is left as-is: the
/// planner already refused a non-deterministic *call* as a partition key, but
/// a deterministic compound expression is not disambiguated by this printer.
fn is_bare_identifier(text: &str) -> bool {
    !text.is_empty()
        && !text.contains('.')
        && !text.contains('(')
        && text.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// Print `node`, qualifying it to `base_alias` when it is a bare column
/// reference (`docs/specs/multi_backend.md`: "base-table references in the
/// outer select are qualified to the bound source's alias").
fn print_qualified_to_base(
    node: &SyntaxNode,
    base_alias: &str,
    ctx: &PrintContext,
    out: &mut String,
) {
    let text = print_trimmed(node, ctx);
    if is_bare_identifier(&text) {
        out.push_str(base_alias);
        out.push('.');
        out.push_str(&text);
    } else {
        out.push_str(&text);
    }
}

/// The synthesised join's null-safe equality spelling, driven entirely by
/// `BackendCapabilities` — never by a `SqlDialect` match
/// (`CLAUDE.md` §"Function-registry single ownership").
fn null_safe_eq(caps: &BackendCapabilities, lhs: &str, rhs: &str) -> String {
    match caps.null_safe_equality {
        NullSafeEqualitySpelling::IsNotDistinctFrom => {
            format!("{lhs} IS NOT DISTINCT FROM {rhs}")
        }
        NullSafeEqualitySpelling::Spaceship => format!("{lhs} <=> {rhs}"),
    }
}

thread_local! {
    /// Stack of active node-substitution tables for `print_node`'s
    /// substitution check. A frame is pushed by `with_active_substitutions`
    /// for the duration of one nested `print_node` call and popped on
    /// return; only the top frame is consulted, so nesting (not currently
    /// exercised — restructure emission never recurses into another
    /// restructure) shadows correctly rather than leaking a stale table.
    static ACTIVE_SUBSTITUTIONS: RefCell<Vec<Vec<(SyntaxNode, String)>>> =
        const { RefCell::new(Vec::new()) };

    /// Stack of active call-position overrides — the mirror of
    /// `ACTIVE_SUBSTITUTIONS` for a call whose *position*, not its text, a
    /// restructure has changed. A `Rowan` `.clone()` of a `FUNCTION_CALL`
    /// node embedded in a synthesised CTE (`restructure::plan_window_to_cte`,
    /// `plan_analytic_to_cte`) is a cheap handle into the *original* tree —
    /// its `.parent()` still resolves to the source query block, still
    /// carrying whatever `OVER`-clause sibling node (or lack of one) the call had
    /// there. Re-deriving position from that stale location would answer the
    /// wrong question: the whole point of a statement-level restructure is
    /// that the call now occupies a *different* position in the printed SQL
    /// (aggregate position inside a `WindowToCte` CTE; window position
    /// inside an `AnalyticToCte` CTE). `restructure::plan` already knows
    /// which position a call is being moved into — it decided the shape of
    /// the CTE it's embedding the call in — so that known position is
    /// pushed here for the duration of printing that one embedded call, and
    /// `emit_registered_function` consults it before ever calling
    /// `position::classify`. This still respects "position is decided once,
    /// by the compile path" (`docs/specs/multi_backend.md` §"Emission is
    /// scoped to call position") — the compile path's restructure planner is
    /// the one deciding it, the printer only carries it through.
    static ACTIVE_POSITION_OVERRIDES: RefCell<Vec<Vec<(SyntaxNode, Position)>>> =
        const { RefCell::new(Vec::new()) };
}

/// RAII guard that pops its substitution frame on drop — unconditionally,
/// including on an early return or panic inside the `f` passed to
/// `with_active_substitutions`. A manual push/pop pair would leave a stale
/// frame on the thread-local stack if `f` ever stopped executing early,
/// silently corrupting substitution for whatever prints next on this thread.
struct SubstitutionFrameGuard;

impl Drop for SubstitutionFrameGuard {
    fn drop(&mut self) {
        ACTIVE_SUBSTITUTIONS.with(|cell| {
            cell.borrow_mut().pop();
        });
    }
}

/// Runs `f` with `subs` active as the current node-substitution table: any
/// node `print_node` encounters while `f` runs that exactly matches one of
/// `subs`'s nodes (by `SyntaxNode` identity, i.e. same underlying green node
/// at the same offset — never by printed-text or range comparison) is
/// replaced with its associated text instead of being printed via the
/// ordinary per-kind dispatch. This is how a replaced call substitutes
/// correctly no matter how deeply it is nested inside surrounding operators
/// or other calls: the ordinary recursive printer walks right past
/// everything else and only swaps the matched node.
pub(crate) fn with_active_substitutions<F: FnOnce()>(subs: &[(SyntaxNode, String)], f: F) {
    ACTIVE_SUBSTITUTIONS.with(|cell| cell.borrow_mut().push(subs.to_vec()));
    let _guard = SubstitutionFrameGuard;
    f();
}

/// The active substitution for `node`, if any — see `with_active_substitutions`.
pub(crate) fn active_substitution_for(node: &SyntaxNode) -> Option<String> {
    ACTIVE_SUBSTITUTIONS.with(|cell| {
        cell.borrow().last().and_then(|subs| {
            subs.iter()
                .find(|(n, _)| n == node)
                .map(|(_, text)| text.clone())
        })
    })
}

/// RAII guard that pops its position-override frame on drop — the mirror of
/// `SubstitutionFrameGuard`, for the same unconditional-pop reason.
struct PositionOverrideFrameGuard;

impl Drop for PositionOverrideFrameGuard {
    fn drop(&mut self) {
        ACTIVE_POSITION_OVERRIDES.with(|cell| {
            cell.borrow_mut().pop();
        });
    }
}

/// Runs `f` with `overrides` active as the current call-position table — see
/// `ACTIVE_POSITION_OVERRIDES`. Used by a restructure's CTE-printing path to
/// hand a specific embedded call the position it now occupies in the
/// synthesised SQL, rather than letting `emit_registered_function` re-derive
/// it from the call's stale original tree location.
pub(crate) fn with_position_override<F: FnOnce()>(overrides: &[(SyntaxNode, Position)], f: F) {
    ACTIVE_POSITION_OVERRIDES.with(|cell| cell.borrow_mut().push(overrides.to_vec()));
    let _guard = PositionOverrideFrameGuard;
    f();
}

/// The active position override for `node`, if any — see
/// `with_position_override`.
pub(crate) fn active_position_override_for(node: &SyntaxNode) -> Option<Position> {
    ACTIVE_POSITION_OVERRIDES.with(|cell| {
        cell.borrow()
            .last()
            .and_then(|overrides| overrides.iter().find(|(n, _)| n == node).map(|(_, p)| *p))
    })
}

/// Whether `call_node` is the *entire* content of `expr_node` — the shape
/// where a select item is exactly a replaced call, optionally followed by
/// one more sibling that is itself one of `replaced`'s empty-string
/// swallow entries (`WindowToCte`'s `OVER` clause, folded into `replaced`
/// this way by the caller — see `print_window_to_cte`). Structural sibling
/// walking here is substitution bookkeeping for an already-planned
/// restructure, not position derivation: which node pairs with which call
/// was decided once, at plan time, in `restructure::plan_window_to_cte`.
/// Only in this shape does the printed output-column name need
/// `SelectItem::column_name()`'s engine-implied-name fallback below: a call
/// wrapped in any surrounding operator has no implied name to preserve, and
/// prints through the general substitution path instead.
fn call_is_whole_expression(
    expr_node: &SyntaxNode,
    replaced: &[(SyntaxNode, String)],
    call_node: &SyntaxNode,
) -> bool {
    let mut nodes = expr_node.children();
    if nodes.next().as_ref() != Some(call_node) {
        return false;
    }
    match nodes.next() {
        None => true,
        Some(n) => {
            nodes.next().is_none() && replaced.iter().any(|(rn, rt)| rn == &n && rt.is_empty())
        }
    }
}

/// Print the outer select list, substituting each replaced call for its
/// joined-back value — wherever it occurs inside a select item's expression
/// tree, not only as the item's whole expression — and otherwise printing
/// each item unchanged (qualified to `base_alias` when given).
fn print_restructured_select_list(
    select: &SelectStmt,
    replaced: &[(SyntaxNode, String)],
    base_alias: Option<&str>,
    ctx: &PrintContext,
    out: &mut String,
) {
    let Some(list) = select.select_list() else {
        return;
    };
    let mut first = true;
    for entry in list.entries() {
        if !first {
            out.push_str(", ");
        }
        first = false;
        match entry {
            SelectEntry::Item(item) => {
                print_restructured_select_item(&item, replaced, base_alias, ctx, out);
            }
            SelectEntry::Spread(spread) => {
                out.push_str(&print_trimmed(spread.syntax(), ctx));
            }
        }
    }
}

/// Print one select item under a statement-level restructure.
///
/// An item whose entire expression is exactly a replaced call
/// (`call_is_whole_expression`) is swapped for its joined-back reference
/// directly, preserving the engine-implied output-column name via
/// `SelectItem::column_name()` when the author wrote no explicit alias —
/// this is the shape admissibility guarantees exists for every restructure
/// (`docs/specs/multi_backend.md` §"Statement-level lowering", admissibility
/// rule 2: every occurrence is in the select list), so every replaced call
/// is reachable from exactly one select item this way or the general path
/// below; there is no third shape left over that would need a refusal here.
///
/// Every other item — the call wrapped in arithmetic, nested inside another
/// call's argument list, … — is printed through the ordinary recursive
/// printer (`print_node`, via `print_qualified_to_base`/`print_trimmed`)
/// with `replaced` active as a substitution table
/// (`with_active_substitutions`): the matched call (and, for `WindowToCte`,
/// its trailing `OVER` clause, already folded into `replaced` as an
/// empty-string entry by the caller) is swapped in place, while every
/// surrounding operator, argument, or function wrapper prints unchanged.
/// `SelectItem` exposes no public accessor for its underlying `SyntaxNode`
/// for the whole-expression case (only `smelt-parser`'s own printer may
/// reach it), so that branch reconstructs the item from its typed accessors
/// rather than reprinting the node wholesale.
fn print_restructured_select_item(
    item: &SelectItem,
    replaced: &[(SyntaxNode, String)],
    base_alias: Option<&str>,
    ctx: &PrintContext,
    out: &mut String,
) {
    let Some(expr) = item.expression() else {
        return;
    };
    let expr_node = expr.syntax();

    if let Some((_, replacement_text)) = replaced
        .iter()
        .find(|(n, _)| call_is_whole_expression(expr_node, replaced, n))
    {
        out.push_str(replacement_text);
        if let Some(alias) = item.alias_token_text().or_else(|| item.column_name()) {
            out.push_str(" AS ");
            out.push_str(&alias);
        }
        return;
    }

    with_active_substitutions(replaced, || match base_alias {
        Some(base_alias) => print_qualified_to_base(expr_node, base_alias, ctx, out),
        None => out.push_str(&print_trimmed(expr_node, ctx)),
    });
    if let Some(alias) = item.alias_token_text() {
        out.push_str(" AS ");
        out.push_str(&alias);
    }
}

/// Print `block`'s own `GROUP BY` / `HAVING` / `ORDER BY` / `LIMIT` clauses
/// verbatim, in that order — the parts of the original query block a
/// restructure leaves untouched (admissibility already refuses a plan whose
/// affected call occurs in any of them).
fn print_trailing_clauses(block: &SyntaxNode, ctx: &PrintContext, out: &mut String) {
    for kind in [
        SyntaxKind::GROUP_BY_CLAUSE,
        SyntaxKind::HAVING_CLAUSE,
        SyntaxKind::ORDER_BY_CLAUSE,
        SyntaxKind::LIMIT_CLAUSE,
    ] {
        if let Some(clause) = block.children().find(|n| n.kind() == kind) {
            out.push(' ');
            out.push_str(&print_trimmed(&clause, ctx));
        }
    }
}

fn print_window_to_cte(
    block: &SyntaxNode,
    base: &BoundSource,
    groups: &[GroupBinding],
    ctx: &PrintContext,
    out: &mut String,
) {
    let Some(select) = SelectStmt::cast(block.clone()) else {
        // The planner only ever produces a plan for a castable SELECT_STMT
        // (`plan_block` casts it before building anything); this path is
        // unreachable in practice.
        print_children(block, ctx, out);
        return;
    };

    out.push_str("WITH ");
    print_author_with_items(block, ctx, out);

    out.push_str(&base.alias);
    out.push_str(" AS (SELECT * ");
    out.push_str(&print_trimmed(&base.from, ctx));
    if let Some(pred) = &base.where_predicate {
        out.push_str(" WHERE ");
        out.push_str(&print_trimmed(pred, ctx));
    }
    out.push(')');

    for group in groups {
        out.push_str(", ");
        out.push_str(&group.cte_name);
        out.push_str(" AS (SELECT ");
        for (i, key) in group.partition_keys.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&print_trimmed(key, ctx));
            out.push_str(&format!(" AS __smelt_k{i}"));
        }
        if !group.partition_keys.is_empty() {
            out.push_str(", ");
        }
        for (i, call) in group.calls.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            // The aggregate form is the call's own text with its OVER clause
            // dropped — the OVER clause is a *sibling* of the FUNCTION_CALL
            // node (parsed that way; see `restructure::partition_keys_of`),
            // never a child, so printing the call node alone already omits
            // it. The call now occupies aggregate position in this CTE —
            // pushed as an override so `emit_registered_function` looks its
            // emission up there rather than re-deriving `WholePartitionWindow`
            // from the clone's still-attached original `OVER`-clause sibling
            // (`ACTIVE_POSITION_OVERRIDES`).
            with_position_override(&[(call.call.clone(), Position::Aggregate)], || {
                out.push_str(&print_trimmed(&call.call, ctx));
            });
            out.push_str(" AS ");
            out.push_str(&call.value_column);
        }
        out.push_str(" FROM ");
        out.push_str(&base.alias);
        if !group.partition_keys.is_empty() {
            out.push_str(" GROUP BY ");
            for i in 0..group.partition_keys.len() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&format!("__smelt_k{i}"));
            }
        }
        out.push(')');
    }

    out.push(' ');
    out.push_str("SELECT ");
    // Each call's `OVER` clause (`call.over_clause`, resolved once at plan
    // time by `restructure::plan_window_to_cte` — never re-derived here) is
    // folded into `replaced` as an empty-string entry, so the general
    // substitution path (`with_active_substitutions`) swallows it
    // automatically wherever the call is nested, exactly as the
    // whole-expression path already does via `call_is_whole_expression`.
    let replaced: Vec<(SyntaxNode, String)> = groups
        .iter()
        .flat_map(|group| {
            group.calls.iter().flat_map(move |call| {
                let mut entries = vec![(
                    call.call.clone(),
                    format!("{}.{}", group.cte_name, call.value_column),
                )];
                if let Some(over_clause) = call.over_clause.clone() {
                    entries.push((over_clause, String::new()));
                }
                entries
            })
        })
        .collect();
    print_restructured_select_list(&select, &replaced, Some(&base.alias), ctx, out);

    out.push_str(" FROM ");
    out.push_str(&base.alias);
    for group in groups {
        if group.partition_keys.is_empty() {
            // A window with no PARTITION BY degenerates to a one-row CTE,
            // joined with a CROSS JOIN.
            out.push_str(" CROSS JOIN ");
            out.push_str(&group.cte_name);
        } else {
            out.push_str(" JOIN ");
            out.push_str(&group.cte_name);
            out.push_str(" ON ");
            for (i, key) in group.partition_keys.iter().enumerate() {
                if i > 0 {
                    out.push_str(" AND ");
                }
                let mut lhs = String::new();
                print_qualified_to_base(key, &base.alias, ctx, &mut lhs);
                let rhs = format!("{}.__smelt_k{}", group.cte_name, i);
                out.push_str(&null_safe_eq(ctx.capabilities, &lhs, &rhs));
            }
        }
    }

    print_trailing_clauses(block, ctx, out);
}

fn print_analytic_to_cte(
    block: &SyntaxNode,
    source: &BoundSource,
    group_keys: &[SyntaxNode],
    replacements: &[AnalyticCallReplacement],
    ctx: &PrintContext,
    out: &mut String,
) {
    let Some(select) = SelectStmt::cast(block.clone()) else {
        print_children(block, ctx, out);
        return;
    };

    out.push_str("WITH ");
    print_author_with_items(block, ctx, out);

    out.push_str(&source.alias);
    out.push_str(" AS (SELECT *");
    for repl in replacements {
        out.push_str(", ");
        out.push_str(&analytic_form_text(repl, group_keys, ctx));
        out.push_str(" AS ");
        out.push_str(&repl.value_column);
    }
    out.push(' ');
    out.push_str(&print_trimmed(&source.from, ctx));
    if let Some(pred) = &source.where_predicate {
        out.push_str(" WHERE ");
        out.push_str(&print_trimmed(pred, ctx));
    }
    out.push(')');

    out.push(' ');
    out.push_str("SELECT ");
    let replaced: Vec<(SyntaxNode, String)> = replacements
        .iter()
        .map(|repl| {
            (
                repl.call.clone(),
                format!("ANY_VALUE({})", repl.value_column),
            )
        })
        .collect();
    print_restructured_select_list(&select, &replaced, None, ctx, out);

    out.push_str(" FROM ");
    out.push_str(&source.alias);

    print_trailing_clauses(block, ctx, out);
}

/// The analytic form `PERCENTILE_CONT(<sort key>, <fraction>) OVER
/// (PARTITION BY <group keys>)` for one `AnalyticToCte` replacement. `repl`'s
/// own `FUNCTION_CALL` node (the ordered-set aggregate with its `WITHIN
/// GROUP` clause) supplies the fraction argument and sort key; `invert`
/// (`fraction_complement`) applies `1 - f` for a `DESC` sort key
/// (`docs/specs/multi_backend.md`: "A `DESC` sort key inverts the
/// fraction").
fn analytic_form_text(
    repl: &AnalyticCallReplacement,
    group_keys: &[SyntaxNode],
    ctx: &PrintContext,
) -> String {
    let call = &repl.call;
    let canonical = FunctionCall::cast(call.clone())
        .and_then(|fc| fc.name())
        .as_deref()
        .and_then(BuiltinRegistry::canonical_name)
        .unwrap_or("")
        .to_string();
    // The call now occupies whole-partition window position in the
    // synthesised CTE — a partition-only `OVER` clause with no `ORDER BY` and
    // no frame (`position::shape_of`'s whole-partition case). Consult the
    // registry at *that* position for the target spelling rather than
    // assuming the canonical name still applies once the call has moved —
    // the mirror-image of `WindowToCte`'s `Position::Aggregate` override
    // above, for the same reason: a call an `AnalyticToCte` restructure
    // moves into window position may need a per-dialect rename there, even
    // though neither of today's registered `AnalyticToCte` entries
    // (`PERCENTILE_CONT`/`PERCENTILE_DISC`) happens to need one.
    let name = BuiltinRegistry::resolve(&canonical)
        .map(
            |sig| match sig.emission_at(ctx.dialect.id(), Position::WholePartitionWindow) {
                Emission::Rename(new_name) => new_name.to_string(),
                _ => canonical.clone(),
            },
        )
        .unwrap_or(canonical);

    let fraction = call
        .children()
        .find(|n| n.kind() == SyntaxKind::ARG_LIST)
        .and_then(|al| al.children().find_map(smelt_parser::ast::Expr::cast))
        .map(|e| print_trimmed(e.syntax(), ctx))
        .unwrap_or_default();
    let fraction = if repl.fraction_complement {
        format!("(1 - {fraction})")
    } else {
        fraction
    };

    let sort_key = call
        .children()
        .find(|n| n.kind() == SyntaxKind::WITHIN_GROUP_CLAUSE)
        .and_then(|wg| {
            wg.children()
                .find(|n| n.kind() == SyntaxKind::ORDER_BY_CLAUSE)
        })
        .and_then(OrderByClause::cast)
        .and_then(|ob| ob.items().next())
        .and_then(|item| item.expression())
        .map(|e| print_trimmed(e.syntax(), ctx))
        .unwrap_or_default();

    let mut out = format!("{name}({sort_key}, {fraction}) OVER (");
    if !group_keys.is_empty() {
        out.push_str("PARTITION BY ");
        for (i, key) in group_keys.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&print_trimmed(key, ctx));
        }
    }
    out.push(')');
    out
}
