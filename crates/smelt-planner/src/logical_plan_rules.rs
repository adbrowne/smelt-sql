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
//! [`RuleContext`] carries a [`CanonicalReturnLookup`] used by the expansion
//! rule to resolve a function's canonical return type when emitting CASTs.
//! Future phases will add filter lists (which rules to skip), backend hints,
//! etc.

use std::sync::Arc;

use smelt_types::signatures::BuiltinRegistry;
use smelt_types::DataType;

use crate::logical::{Cardinality, FnId, FunctionProperties, LogicalNode, Plan, Provenance};

// ---------------------------------------------------------------------------
// Public API types
// ---------------------------------------------------------------------------

/// Resolves a function's canonical return type by name.
///
/// Phase 40 introduces this trait so the expansion rule can emit
/// CAST nodes whose target type comes from
/// [`smelt_types::signatures::Signature::canonical_return`] rather than a
/// hardcoded placeholder. The trait abstracts the registry source so tests
/// can inject custom lookups without mutating the global built-in registry.
pub trait CanonicalReturnLookup: Send + Sync {
    /// Return the canonical SQL return type for `fn_id`, or `None` if the
    /// function is not registered or has no declared canonical type.
    fn canonical_return(&self, fn_id: &str) -> Option<DataType>;
}

/// Default lookup that consults the global [`BuiltinRegistry`].
pub struct BuiltinCanonicalReturnLookup;

impl CanonicalReturnLookup for BuiltinCanonicalReturnLookup {
    fn canonical_return(&self, fn_id: &str) -> Option<DataType> {
        BuiltinRegistry::resolve(fn_id).and_then(|sig| sig.canonical_return.clone())
    }
}

/// Lookup that always returns `None`. Used as the default when no registry
/// is supplied (e.g. in legacy tests that don't exercise the cast path).
pub struct EmptyCanonicalReturnLookup;

impl CanonicalReturnLookup for EmptyCanonicalReturnLookup {
    fn canonical_return(&self, _fn_id: &str) -> Option<DataType> {
        None
    }
}

/// Context available to planner rules during a pass.
///
/// The `registry` field exposes a [`CanonicalReturnLookup`] that
/// [`ExpandTransparentFunctionCalls`] uses to resolve CAST target types.
/// Construct with [`RuleContext::default()`] for no-cast tests or
/// [`RuleContext::with_builtins()`] for the production path.
pub struct RuleContext {
    /// Resolves canonical return types by function name.
    pub registry: Arc<dyn CanonicalReturnLookup>,
}

impl Default for RuleContext {
    fn default() -> Self {
        RuleContext {
            registry: Arc::new(EmptyCanonicalReturnLookup),
        }
    }
}

impl RuleContext {
    /// Construct a [`RuleContext`] backed by the global [`BuiltinRegistry`].
    pub fn with_builtins() -> Self {
        RuleContext {
            registry: Arc::new(BuiltinCanonicalReturnLookup),
        }
    }

    /// Construct a [`RuleContext`] with a custom registry resolver.
    pub fn with_registry(registry: Arc<dyn CanonicalReturnLookup>) -> Self {
        RuleContext { registry }
    }
}

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
///
/// Uses [`RuleContext::default()`] (an empty registry). Call sites that need
/// CAST target types resolved from the built-in registry should use
/// [`apply_rules_to_fixed_point_with_ctx`] instead.
pub fn apply_rules_to_fixed_point(plan: Plan, rules: &[Box<dyn PlannerRule>]) -> Plan {
    apply_rules_to_fixed_point_with_ctx(plan, rules, &RuleContext::default())
}

/// Variant of [`apply_rules_to_fixed_point`] that runs the loop with a
/// caller-provided [`RuleContext`].
pub fn apply_rules_to_fixed_point_with_ctx(
    mut plan: Plan,
    rules: &[Box<dyn PlannerRule>],
    ctx: &RuleContext,
) -> Plan {
    loop {
        let mut changed = false;
        for rule in rules {
            match rule.apply(plan.clone(), ctx) {
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
    fn apply(&self, plan: Plan, ctx: &RuleContext) -> RuleResult {
        expand_recursive(plan, ctx)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn expand_recursive(node: Plan, ctx: &RuleContext) -> RuleResult {
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
            let expanded = build_expanded_call(fn_id, provenance, properties, pushed_filter, ctx);
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
        } => expand_select(projections, from, filter, ctx),

        LogicalNode::Cast { inner, target_type } => match expand_recursive(inner.clone(), ctx) {
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
            let lhs_result = expand_recursive(lhs.clone(), ctx);
            let rhs_result = expand_recursive(rhs.clone(), ctx);
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
///
/// When `properties.needs_cast` is `true`, the cast target type is resolved
/// from `ctx.registry.canonical_return(fn_id)`. If the registry has no entry
/// (or no canonical return declared), the cast is omitted entirely — emitting
/// a Cast with no known target would be incorrect.
fn build_expanded_call(
    fn_id: &FnId,
    provenance: &Provenance,
    properties: &FunctionProperties,
    pushed_filter: &Option<Plan>,
    ctx: &RuleContext,
) -> Plan {
    let expanded: Plan = Arc::new(LogicalNode::ExpandedCall {
        fn_id: fn_id.clone(),
        provenance: provenance.clone(),
        properties: properties.clone(),
        pushed_filter: pushed_filter.clone(),
    });

    if !properties.needs_cast {
        return expanded;
    }

    match ctx.registry.canonical_return(fn_id) {
        Some(target_type) => Arc::new(LogicalNode::Cast {
            inner: expanded,
            target_type,
        }),
        None => expanded,
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
fn expand_select(
    projections: &[String],
    from: &Option<Plan>,
    filter: &Option<Plan>,
    ctx: &RuleContext,
) -> RuleResult {
    let from_result = from.as_ref().map(|f| expand_recursive(f.clone(), ctx));
    let filter_result = filter.as_ref().map(|f| expand_recursive(f.clone(), ctx));

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
