//! Phase 32 — Logical-plan rewrite rules.
//!
//! This module defines the [`PlannerRule`] trait, the fixed-point execution
//! loop [`apply_rules_to_fixed_point`], and the first rule:
//! [`ExpandTransparentFunctionCalls`].
//!
//! # Design notes
//!
//! These are **logical-plan-level** rules that operate on the
//! `Arc<LogicalNode>` tree.  They are completely separate from the
//! *graph-level* rules in `crates/smelt-planner/src/rules/`, which operate
//! on the model dependency graph.
//!
//! [`RuleContext`] is intentionally empty in Phase 32.  Future phases will
//! add filter lists (which rules to skip), backend hints, etc.

use std::sync::Arc;

use smelt_types::DataType;

use crate::logical::{Cardinality, FnId, FunctionProperties, LogicalNode, Plan, Provenance};

// ---------------------------------------------------------------------------
// Public API types
// ---------------------------------------------------------------------------

/// Context available to planner rules during a pass.
///
/// Phase 32: intentionally empty.  Phase 33+ will add filter lists and
/// backend configuration.
pub struct RuleContext;

/// The result of applying a [`PlannerRule`] to a plan node.
pub enum RuleResult {
    /// The rule rewrote the plan; the new plan is returned.
    Changed(Plan),
    /// The rule did not modify the plan.
    Unchanged,
}

/// Trait for a single logical-plan rewrite rule.
///
/// Implementations must be `Send + Sync` so they can be stored in
/// `Vec<Box<dyn PlannerRule>>` and used from multiple threads.
pub trait PlannerRule: Send + Sync {
    /// Attempt to apply this rule to `plan`.
    ///
    /// Return [`RuleResult::Changed`] with the new plan if the rule fired,
    /// or [`RuleResult::Unchanged`] if the rule did not apply.
    fn apply(&self, plan: Plan, ctx: &RuleContext) -> RuleResult;
}

// ---------------------------------------------------------------------------
// Fixed-point loop
// ---------------------------------------------------------------------------

/// Build the rule list `smelt build --show-plan` runs over each model's
/// logical plan: filter push-down into transparent calls, expansion of
/// transparent calls, and elimination of unused 1:1 LEFT JOINs.
///
/// Order matters: pushdown must run before expansion. Pushdown matches
/// `Select { from: FunctionCall { transparent: true, .. } }`, while
/// expansion replaces that `FunctionCall` with an `ExpandedCall` marker
/// that pushdown does not match. The same ordering is asserted by the
/// `combined_rule_set_reaches_fixed_point` test in `pushdown_tests.rs`.
pub fn show_plan_rules() -> Vec<Box<dyn PlannerRule>> {
    vec![
        Box::new(PushFilterIntoTransparentFunction),
        Box::new(ExpandTransparentFunctionCalls),
        Box::new(EliminateUnusedLeftJoin),
    ]
}

/// Run all `rules` over `plan` in a fixed-point loop.
///
/// Each pass applies every rule in order; if any rule fires the loop repeats
/// from the beginning with the updated plan.  The loop terminates when a full
/// pass completes without any rule returning [`RuleResult::Changed`].
pub fn apply_rules_to_fixed_point(mut plan: Plan, rules: &[Box<dyn PlannerRule>]) -> Plan {
    loop {
        let mut changed = false;
        for rule in rules {
            match rule.apply(plan.clone(), &RuleContext) {
                RuleResult::Changed(new_plan) => {
                    plan = new_plan;
                    changed = true;
                }
                RuleResult::Unchanged => {}
            }
        }
        if !changed {
            break;
        }
    }
    plan
}

// ---------------------------------------------------------------------------
// Rule: ExpandTransparentFunctionCalls
// ---------------------------------------------------------------------------

/// Expand every `FunctionCall { transparent: true }` into an [`LogicalNode::ExpandedCall`]
/// marker node.
///
/// If the call's [`FunctionProperties::needs_cast`] flag is `true`, the
/// `ExpandedCall` node is additionally wrapped in a [`LogicalNode::Cast`] node
/// so that physical-plan emission can insert the appropriate SQL `CAST(…)`.
///
/// The rule recurses into `Select` and `Cast` children.  It returns
/// [`RuleResult::Unchanged`] when no transparent call is found anywhere in the
/// subtree, allowing the fixed-point loop to terminate.
pub struct ExpandTransparentFunctionCalls;

impl PlannerRule for ExpandTransparentFunctionCalls {
    fn apply(&self, plan: Plan, _ctx: &RuleContext) -> RuleResult {
        expand_recursive(plan)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn expand_recursive(node: Plan) -> RuleResult {
    match node.as_ref() {
        // --- transparent call: expand it ---
        LogicalNode::FunctionCall {
            transparent: true,
            fn_id,
            provenance,
            properties,
            pushed_filter,
            ..
        } => {
            let expanded = build_expanded_call(fn_id, provenance, properties, pushed_filter);
            RuleResult::Changed(expanded)
        }

        // --- opaque or already-expanded: leave intact ---
        LogicalNode::FunctionCall {
            transparent: false, ..
        }
        | LogicalNode::ExpandedCall { .. }
        | LogicalNode::TableRef { .. }
        | LogicalNode::Literal(_) => RuleResult::Unchanged,

        // --- structural nodes: recurse into children ---
        LogicalNode::Select {
            projections,
            from,
            filter,
        } => expand_select(projections, from, filter),

        LogicalNode::Cast { inner, target_type } => match expand_recursive(inner.clone()) {
            RuleResult::Changed(new_inner) => RuleResult::Changed(Arc::new(LogicalNode::Cast {
                inner: new_inner,
                target_type: target_type.clone(),
            })),
            RuleResult::Unchanged => RuleResult::Unchanged,
        },

        // LeftJoin: recurse into both children.
        LogicalNode::LeftJoin {
            lhs,
            rhs,
            join_columns,
            cardinality,
            output_columns,
        } => {
            let lhs_result = expand_recursive(lhs.clone());
            let rhs_result = expand_recursive(rhs.clone());
            let lhs_changed = matches!(lhs_result, RuleResult::Changed(_));
            let rhs_changed = matches!(rhs_result, RuleResult::Changed(_));
            if lhs_changed || rhs_changed {
                let new_lhs = match lhs_result {
                    RuleResult::Changed(p) => p,
                    _ => lhs.clone(),
                };
                let new_rhs = match rhs_result {
                    RuleResult::Changed(p) => p,
                    _ => rhs.clone(),
                };
                RuleResult::Changed(Arc::new(LogicalNode::LeftJoin {
                    lhs: new_lhs,
                    rhs: new_rhs,
                    join_columns: join_columns.clone(),
                    cardinality: cardinality.clone(),
                    output_columns: output_columns.clone(),
                }))
            } else {
                RuleResult::Unchanged
            }
        }
    }
}

/// Construct the `ExpandedCall` (plus optional `Cast` wrapper) for a transparent call.
fn build_expanded_call(
    fn_id: &FnId,
    provenance: &Provenance,
    properties: &FunctionProperties,
    pushed_filter: &Option<Plan>,
) -> Plan {
    let expanded: Plan = Arc::new(LogicalNode::ExpandedCall {
        fn_id: fn_id.clone(),
        provenance: provenance.clone(),
        properties: properties.clone(),
        pushed_filter: pushed_filter.clone(),
    });

    if properties.needs_cast {
        // Phase 32: use BigInt as a placeholder target type.
        // Phase 33+ will resolve the actual return type from the function registry.
        Arc::new(LogicalNode::Cast {
            inner: expanded,
            target_type: DataType::BigInt,
        })
    } else {
        expanded
    }
}

// ---------------------------------------------------------------------------
// Rule: PushFilterIntoTransparentFunction
// ---------------------------------------------------------------------------

/// Push a `WHERE` predicate from an enclosing `Select` into a transparent
/// `FunctionCall` as a `pushed_filter` hint.
///
/// # Conditions for firing
///
/// All of the following must hold:
/// 1. The plan is a `Select { filter: Some(pred), from: Some(FunctionCall { .. }) }`.
/// 2. The `FunctionCall` is transparent (`transparent: true`).
/// 3. The `FunctionCall` has `Declared` (not `Unknown`) provenance — opaque
///    provenance means we don't know whether it's safe to push.
/// 4. The `FunctionCall`'s `properties.deterministic` is `true` — non-deterministic
///    functions may return different rows on each invocation, making pre-filtering
///    incorrect in general.
/// 5. `pushed_filter` on the `FunctionCall` is `None` — once a filter is pushed
///    the rule is idempotent and will not push a second time.
///
/// # Effect
///
/// * The `FunctionCall` node receives `pushed_filter: Some(pred)`.
/// * The enclosing `Select`'s `filter` is set to `None` (predicate consumed).
pub struct PushFilterIntoTransparentFunction;

impl PlannerRule for PushFilterIntoTransparentFunction {
    fn apply(&self, plan: Plan, _ctx: &RuleContext) -> RuleResult {
        let LogicalNode::Select {
            ref projections,
            from: Some(ref from_node),
            filter: Some(ref pred),
        } = *plan
        else {
            return RuleResult::Unchanged;
        };

        let LogicalNode::FunctionCall {
            ref fn_id,
            ref args,
            transparent,
            ref provenance,
            ref properties,
            ref pushed_filter,
        } = *from_node.as_ref()
        else {
            return RuleResult::Unchanged;
        };

        // Guard: must be transparent.
        if !transparent {
            return RuleResult::Unchanged;
        }

        // Guard: must have declared provenance.
        if !matches!(provenance, Provenance::Declared(_)) {
            return RuleResult::Unchanged;
        }

        // Guard: must be deterministic.
        if !properties.deterministic {
            return RuleResult::Unchanged;
        }

        // Guard: idempotent — don't push if already pushed.
        if pushed_filter.is_some() {
            return RuleResult::Unchanged;
        }

        // Build the updated FunctionCall with the filter pushed in.
        let new_call = Arc::new(LogicalNode::FunctionCall {
            fn_id: fn_id.clone(),
            args: args.clone(),
            transparent,
            provenance: provenance.clone(),
            properties: properties.clone(),
            pushed_filter: Some(pred.clone()),
        });

        // Build the new Select with the filter cleared.
        let new_select = Arc::new(LogicalNode::Select {
            projections: projections.clone(),
            from: Some(new_call),
            filter: None,
        });

        RuleResult::Changed(new_select)
    }
}

// ---------------------------------------------------------------------------
// Rule: EliminateUnusedLeftJoin
// ---------------------------------------------------------------------------

/// Elide a `LeftJoin` whose RHS columns are never consumed by the parent
/// `Select`'s projection list.
///
/// # Conditions for firing
///
/// All of the following must hold:
/// 1. The plan is a `Select { projections, from: Some(LeftJoin { .. }), .. }`.
/// 2. The `LeftJoin` has `cardinality == OneToOne`.
///    — `OneToMany` is never safe: dropping the join could change row counts.
/// 3. None of the `LeftJoin::output_columns` appear in `projections`.
///    — If any RHS column is projected, it must be produced.
///
/// # Effect
///
/// The `Select`'s `from` is replaced with the `LeftJoin`'s `lhs`, dropping
/// the join and the RHS entirely.
///
/// # Soundness caveat (§20E)
///
/// The rule trusts the declared cardinality without verifying it against
/// actual data. Mismatched declarations will silently produce incorrect results.
pub struct EliminateUnusedLeftJoin;

impl PlannerRule for EliminateUnusedLeftJoin {
    fn apply(&self, plan: Plan, _ctx: &RuleContext) -> RuleResult {
        let LogicalNode::Select {
            ref projections,
            from: Some(ref from_node),
            ref filter,
        } = *plan
        else {
            return RuleResult::Unchanged;
        };

        let LogicalNode::LeftJoin {
            ref lhs,
            cardinality: Cardinality::OneToOne,
            ref output_columns,
            ..
        } = *from_node.as_ref()
        else {
            return RuleResult::Unchanged;
        };

        // Check that no output column from the RHS appears in the projection list.
        let any_rhs_used = output_columns
            .iter()
            .any(|col| projections.iter().any(|p| p == col));

        if any_rhs_used {
            return RuleResult::Unchanged;
        }

        // Safe to elide: replace from with lhs only.
        let new_plan = Arc::new(LogicalNode::Select {
            projections: projections.clone(),
            from: Some(lhs.clone()),
            filter: filter.clone(),
        });

        RuleResult::Changed(new_plan)
    }
}

/// Recurse into a `Select` node's `from` and `filter` children.
fn expand_select(projections: &[String], from: &Option<Plan>, filter: &Option<Plan>) -> RuleResult {
    let from_result = from.as_ref().map(|f| expand_recursive(f.clone()));
    let filter_result = filter.as_ref().map(|f| expand_recursive(f.clone()));

    let from_changed = from_result
        .as_ref()
        .map(|r| matches!(r, RuleResult::Changed(_)))
        .unwrap_or(false);
    let filter_changed = filter_result
        .as_ref()
        .map(|r| matches!(r, RuleResult::Changed(_)))
        .unwrap_or(false);

    if from_changed || filter_changed {
        let new_from = match from_result {
            Some(RuleResult::Changed(p)) => Some(p),
            _ => from.clone(),
        };
        let new_filter = match filter_result {
            Some(RuleResult::Changed(p)) => Some(p),
            _ => filter.clone(),
        };
        RuleResult::Changed(Arc::new(LogicalNode::Select {
            projections: projections.to_vec(),
            from: new_from,
            filter: new_filter,
        }))
    } else {
        RuleResult::Unchanged
    }
}
