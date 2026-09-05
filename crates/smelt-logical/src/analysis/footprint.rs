//! Footprint reflection — the write-scope dual of the per-source read-bound
//! derivation (`docs/specs/model_properties.md` §"Footprint reflection /
//! bounded write footprint").
//!
//! [`super::source_bounds::derive_model_bounds`] asks how far *outside* the
//! run window a **read** of each source must reach; [`reflect_footprint`]
//! asks the dual question: how far across the **output's own partition
//! column** must a *write* triggered by an input delta spread.  The two are
//! structurally the same reach computation — this module runs the exact same
//! walk-backed derivation (series/parallel composition, shared interval
//! parser) and reflects its verdict rather than re-deriving anything from
//! text:
//!
//! - a `Bounded { before, after }` read reach reflects to the mirror
//!   `Bounded { before: after, after: before }` on the output axis — a
//!   source delta at time `t` writes output over `[t − read.after,
//!   t + read.before]`;
//! - an `Unbounded` / `NotDerivable` read bound reflects to `Unbounded` /
//!   `NotDerivable` respectively — fail-closed, never a guessed mirror;
//! - a stored **trajectory column** — a running/cumulative fold over the
//!   output axis, whose stored value is still mutable arbitrarily far
//!   downstream under late input — reflects to `Unbounded` even when the
//!   read reach is bounded: this is the canonical case the read-side mirror
//!   cannot express.

use std::collections::HashMap;

use serde::Serialize;

use crate::analysis::source_bounds::{derive_model_bounds, BoundContext, BoundResult, Seconds};
use crate::analysis::walk::{LeafInput, NodeCx, OpNode, QueryTree, Transfer};
use crate::analysis::{
    item_expr, select_stmt_items, window_has_bounded_range_interval_frame, SelectItemKind,
};

/// The derived write footprint for one source reference — the write-scope
/// dual of [`BoundResult`], sharing its three-way verdict shape.  Where
/// `BoundResult::Bounded` names the *source's* partition column (the axis
/// the read is clamped on), `FootprintResult::Bounded` names the *output's*
/// partition column (the axis the write spreads across).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FootprintResult {
    /// A delta of the source at time `t` writes output partitions over
    /// `[t − before, t + after]` on the output's own partition column.
    Bounded {
        output_partition_col: String,
        before: Seconds,
        after: Seconds,
    },
    /// A delta can rewrite arbitrarily distant output partitions (e.g. a
    /// stored trajectory column under late data).
    Unbounded,
    /// The write scope cannot be derived from the SQL patterns present —
    /// absence of a proof is a rejection, never an optimistic mirror.
    NotDerivable,
}

/// Derive the per-source write footprint of a model: for each `smelt.<path>`
/// source in `ctx`, how far across `output_partition_col` (the output's own
/// partition axis) a write triggered by that source's delta must spread.
///
/// Reuses the walk-backed read-reach derivation
/// ([`derive_model_bounds`] — same series/parallel composition, same
/// interval parser) and reflects its verdict; the only footprint-specific
/// derivation is the trajectory-column detection, itself run over the same
/// composition walk with a per-scope leaf classifier.
///
/// `output_partition_col: None` (a key-grain output with no partition axis)
/// yields `NotDerivable` for every source with a bounded read reach — the
/// footprint question is posed against an output axis, and there is none to
/// bound the write on.
pub fn reflect_footprint(
    sql: &str,
    ctx: &BoundContext,
    output_partition_col: Option<&str>,
) -> HashMap<String, FootprintResult> {
    let bounds = derive_model_bounds(sql, ctx);
    let trajectory = match output_partition_col {
        Some(axis) => model_has_trajectory_column(sql, axis),
        None => false,
    };
    bounds
        .into_iter()
        .map(|(source, bound)| {
            let fp = match bound {
                // Fail-closed reflections: never a guessed mirror.
                BoundResult::NotDerivable => FootprintResult::NotDerivable,
                BoundResult::Unbounded => FootprintResult::Unbounded,
                BoundResult::Bounded { before, after, .. } => match output_partition_col {
                    None => FootprintResult::NotDerivable,
                    Some(_) if trajectory => FootprintResult::Unbounded,
                    Some(axis) => FootprintResult::Bounded {
                        output_partition_col: axis.to_string(),
                        // The mirror: scan (before, after) ⇒ write
                        // (after, before) — a delta at t writes output over
                        // [t − scan.after, t + scan.before].
                        before: after,
                        after: before,
                    },
                },
            };
            (source, fp)
        })
        .collect()
}

/// Whether any SELECT scope of the model stores a trajectory column — a
/// running/cumulative fold over the output axis.  Composed over the shared
/// bottom-up walk (parallel OR across scopes); for a tree the walk cannot
/// normalize, falls back to classifying every `SelectStmt` scope of the
/// parsed CST directly (mirroring [`derive_model_bounds`]'s whole-text
/// fallback — coverage never degrades below the flat enumeration).
fn model_has_trajectory_column(sql: &str, output_partition_col: &str) -> bool {
    match QueryTree::from_sql(sql) {
        Some(tree) if !tree.root.has_unsupported() => crate::analysis::walk::walk(
            &tree,
            &TrajectoryTransfer {
                axis: output_partition_col,
            },
        ),
        _ => {
            // Fallback enumeration for shapes the tree normalization cannot
            // model: every SELECT scope in the parsed CST, each judged by
            // its own select-list items only.
            let stripped = crate::types::Frontmatter::strip(sql);
            let parse = smelt_parser::parse(stripped);
            parse
                .syntax()
                .descendants()
                .filter_map(smelt_parser::SelectStmt::cast)
                .any(|select| scope_has_running_fold_over_axis(&select, output_partition_col))
        }
    }
}

/// The trajectory-detection transfer over the composition walk: each SELECT
/// node contributes its own scope's verdict; children compose in parallel
/// (OR — any scope's trajectory column makes the stored model a trajectory).
///
/// `ctes` and `inputs` fold in unconditionally, same as every other
/// child-tail consumer. An `expr_scopes` child folds in only when its own
/// `ExprScope::range` sits inside one of this scope's own select-list
/// items' expression range — i.e. only when the subquery's value actually
/// flows into a stored output column. Trajectory is a property of a
/// *stored* column (`docs/specs/model_properties.md` §"Footprint reflection
/// / bounded write footprint": "a stored trajectory column … whose stored
/// value is still mutable arbitrarily far downstream"); a running fold
/// buried in a `WHERE`/`HAVING`/`QUALIFY`/`ORDER BY` scalar or `EXISTS`/`IN`/
/// quantified subquery never becomes a stored column of this scope, so it
/// contributes no trajectory here (`window_inside_a_where_subquery_is_not_a_trajectory_of_the_outer_select`
/// pins this: an unconditional whole-slice fold would misclassify it).
struct TrajectoryTransfer<'a> {
    axis: &'a str,
}

impl Transfer for TrajectoryTransfer<'_> {
    type Verdict = bool;

    fn leaf(&self, _leaf: &LeafInput<'_>, _cx: &NodeCx) -> bool {
        false
    }

    fn operator(&self, op: &OpNode<'_>, children: &[bool], _cx: &NodeCx) -> bool {
        match op {
            // Unreachable in practice: `model_has_trajectory_column` routes
            // unsupported trees to the CST fallback before walking.
            // `children` is always empty here (no `Unsupported` children).
            OpNode::Unsupported { .. } => children.iter().any(|c| *c),
            OpNode::SetOp(_) => children.iter().any(|c| *c),
            OpNode::Select(sn) => {
                let n = sn.ctes.len() + sn.inputs.len();
                let child_hit = children[..n].iter().any(|c| *c);
                let expr_scope_hit = sn
                    .select
                    .select_list()
                    .map(|select_list| {
                        sn.expr_scopes.iter().zip(&children[n..]).any(|(es, hit)| {
                            *hit && select_list.items().any(|item| {
                                item.expression().is_some_and(|expr| {
                                    expr.syntax().text_range().contains_range(es.range)
                                })
                            })
                        })
                    })
                    .unwrap_or(false);
                child_hit
                    || expr_scope_hit
                    || scope_has_running_fold_over_axis(&sn.select, self.axis)
            }
        }
    }
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): invoked by [`TrajectoryTransfer`] over one SELECT scope's own
/// parsed select-list items (never the surrounding query, never raw text).
///
/// A scope stores a trajectory column when a select item is an aggregate
/// combiner applied in a running window over the output axis: an `OVER`
/// whose `ORDER BY` leads with `axis` and whose frame is not a bounded
/// `RANGE BETWEEN INTERVAL … ` one (a bounded interval frame has a finite
/// reach the read-bound derivation already picks up, so its mirror is the
/// correct footprint).  The combiner is classified through the same
/// [`combiner_discriminants`] table the walk's `PropertyVector::discriminants`
/// carries: an inverse-needing or order/value-monotone running fold
/// (`MIN`/`MAX`/`ARG_MAX`), a monoid running total (`SUM`/`COUNT` — the
/// canonical cumulative case), and the holistic fail-closed default alike
/// are trajectories once folded along the axis — a late input at `t`
/// changes the stored value of output rows arbitrarily far after `t`.
fn scope_has_running_fold_over_axis(select: &smelt_parser::SelectStmt, axis: &str) -> bool {
    let Some(items) = select_stmt_items(select) else {
        return false;
    };
    for item in &items {
        // Only combiner-fold items can be trajectories; a plain grouping key
        // stores no fold state.
        if matches!(item, SelectItemKind::GroupByKey { .. }) {
            continue;
        }
        let expr = item_expr(item);
        let Some(window) = expr.window_spec() else {
            // A non-windowed aggregate folds within its GROUP BY group; its
            // reach is the read bound's business, not a trajectory.
            continue;
        };
        // The fold must run ALONG the output axis: an ORDER BY led by the
        // axis (`ORDER BY ALL` is treated as containing it — fail-closed).
        let ordered_by_axis = match window.order_by() {
            Some(order_by) => {
                order_by.is_all()
                    || order_by
                        .items()
                        .any(|item| item.expression().is_some_and(|e| expr_is_column(&e, axis)))
            }
            // No ORDER BY: a per-partition aggregate, not a running fold.
            None => false,
        };
        if !ordered_by_axis {
            continue;
        }
        // A bounded RANGE-interval frame has finite reach — the read-bound
        // derivation picks it up and the mirror is the correct footprint.
        // Any other frame (none — the implicit UNBOUNDED PRECEDING default —
        // ROWS frames, unbounded RANGE) is a running fold.
        if window_has_bounded_range_interval_frame(&window) {
            continue;
        }
        // Only aggregate combiners are trajectories in running position —
        // every discriminant class (inverse-needing, monotone, monoid, and
        // the holistic fail-closed default) qualifies, so aggregate-ness is
        // the whole test.  A non-aggregate window function (`ROW_NUMBER`,
        // `LAG`) never routes through this arm; those are the read-bound
        // derivation's business.
        let Some(func) = expr
            .as_function_call()
            .and_then(|f| f.name())
            .and_then(|n| smelt_types::SqlFunction::from_name(&n.to_uppercase()))
        else {
            continue;
        };
        if !func.is_aggregate() {
            continue;
        }
        return true;
    }
    false
}

/// Leaf classifier (`docs/specs/architecture.md` §"Property composition walk
/// rule"): sub-helper of [`scope_has_running_fold_over_axis`], scoped to one
/// already-parsed ORDER BY item expression — true when it is a bare or
/// table-qualified reference to `column` (case-insensitive).
fn expr_is_column(expr: &smelt_parser::Expr, column: &str) -> bool {
    expr.as_column_ref()
        .is_some_and(|c| c.name().eq_ignore_ascii_case(column))
}
