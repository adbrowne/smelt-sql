//! Phase 32 — `PlannerRule` trait and `ExpandTransparentFunctionCalls` rule tests.
//!
//! These tests are pure (no Salsa dependency) — they construct `Arc<LogicalNode>`
//! trees directly and drive the rule API through the public interface in
//! `smelt_planner::logical_plan_rules`.

use std::sync::Arc;

use smelt_planner::logical::{FunctionProperties, LogicalNode, Provenance};
use smelt_planner::logical_plan_rules::{
    apply_rules_to_fixed_point, ExpandTransparentFunctionCalls, PlannerRule, RuleContext,
    RuleResult,
};
use smelt_types::DataType;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_transparent_call(fn_id: &str) -> Arc<LogicalNode> {
    Arc::new(LogicalNode::FunctionCall {
        fn_id: fn_id.to_string(),
        args: vec![],
        transparent: true,
        provenance: Provenance::Unknown,
        properties: FunctionProperties::default(),
        pushed_filter: None,
        body: None,
    })
}

fn make_opaque_call(fn_id: &str) -> Arc<LogicalNode> {
    Arc::new(LogicalNode::FunctionCall {
        fn_id: fn_id.to_string(),
        args: vec![],
        transparent: false,
        provenance: Provenance::Unknown,
        properties: FunctionProperties::default(),
        pushed_filter: None,
        body: None,
    })
}

fn make_transparent_call_with_provenance(fn_id: &str, provenance: Provenance) -> Arc<LogicalNode> {
    Arc::new(LogicalNode::FunctionCall {
        fn_id: fn_id.to_string(),
        args: vec![],
        transparent: true,
        provenance,
        properties: FunctionProperties::default(),
        pushed_filter: None,
        body: None,
    })
}

fn make_needs_cast_call(fn_id: &str) -> Arc<LogicalNode> {
    Arc::new(LogicalNode::FunctionCall {
        fn_id: fn_id.to_string(),
        args: vec![],
        transparent: true,
        provenance: Provenance::Unknown,
        properties: FunctionProperties {
            needs_cast: true,
            ..FunctionProperties::default()
        },
        pushed_filter: None,
        body: None,
    })
}

/// Returns true if any node in the tree is a FunctionCall with transparent=true.
fn has_transparent_call(node: &Arc<LogicalNode>) -> bool {
    match node.as_ref() {
        LogicalNode::FunctionCall {
            transparent: true, ..
        } => true,
        LogicalNode::FunctionCall {
            transparent: false, ..
        } => false,
        LogicalNode::ExpandedCall { body, .. } => {
            body.as_ref().map(has_transparent_call).unwrap_or(false)
        }
        LogicalNode::TableRef { .. } | LogicalNode::Literal(_) | LogicalNode::Raw { .. } => false,
        LogicalNode::Select { from, filter, .. } => {
            from.as_ref().map(has_transparent_call).unwrap_or(false)
                || filter.as_ref().map(has_transparent_call).unwrap_or(false)
        }
        LogicalNode::Cast { inner, .. } => has_transparent_call(inner),
        LogicalNode::Tagged { inner, .. } => has_transparent_call(inner),
        LogicalNode::SpliceList(items) => items.iter().any(has_transparent_call),
        LogicalNode::LeftJoin { lhs, rhs, .. } => {
            has_transparent_call(lhs) || has_transparent_call(rhs)
        }
    }
}

// ---------------------------------------------------------------------------
// Test 1 — PlannerRule trait shape
// ---------------------------------------------------------------------------
//
// A `StubRule` that always returns `Unchanged` satisfies the `PlannerRule` trait.

struct StubRule;

impl PlannerRule for StubRule {
    fn apply(&self, _plan: Arc<LogicalNode>, _ctx: &RuleContext) -> RuleResult {
        RuleResult::Unchanged
    }
}

#[test]
fn planner_rule_trait_shape() {
    let plan = make_opaque_call("some_fn");
    let rule = StubRule;
    let ctx = RuleContext::default();
    match rule.apply(plan, &ctx) {
        RuleResult::Unchanged => {}
        RuleResult::Changed(_) => panic!("StubRule should return Unchanged"),
    }
}

// ---------------------------------------------------------------------------
// Test 2 — expansion rule replaces transparent calls
// ---------------------------------------------------------------------------

#[test]
fn expansion_rule_replaces_transparent_calls() {
    let plan = make_transparent_call("my_fn");
    let rule = ExpandTransparentFunctionCalls;
    let ctx = RuleContext::default();

    let result = rule.apply(plan, &ctx);
    let new_plan = match result {
        RuleResult::Changed(p) => p,
        RuleResult::Unchanged => panic!("Expected Changed result for transparent call"),
    };

    assert!(
        !has_transparent_call(&new_plan),
        "Result must not contain any FunctionCall{{transparent: true}} nodes; got: {new_plan:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 3 — black-box (non-transparent) calls are left intact
// ---------------------------------------------------------------------------

#[test]
fn black_box_calls_left_intact() {
    let plan = make_opaque_call("ext_fn");
    let rule = ExpandTransparentFunctionCalls;
    let ctx = RuleContext::default();

    match rule.apply(plan, &ctx) {
        RuleResult::Unchanged => {}
        RuleResult::Changed(p) => {
            panic!("Expected Unchanged for opaque call, got Changed: {p:?}")
        }
    }
}

// ---------------------------------------------------------------------------
// Test 4 — fixed-point terminates
// ---------------------------------------------------------------------------
//
// First application → Changed; second application on the result → Unchanged.
// (The ExpandedCall marker prevents re-expansion.)

#[test]
fn fixed_point_terminates() {
    let plan = make_transparent_call("my_fn");
    let rule = ExpandTransparentFunctionCalls;
    let ctx = RuleContext::default();

    // First pass: must change.
    let result1 = rule.apply(plan, &ctx);
    let after_first = match result1 {
        RuleResult::Changed(p) => p,
        RuleResult::Unchanged => panic!("Expected Changed on first pass"),
    };

    // Second pass: must NOT change.
    match rule.apply(after_first, &ctx) {
        RuleResult::Unchanged => {}
        RuleResult::Changed(p) => {
            panic!("Expected Unchanged on second pass (fixed-point), got Changed: {p:?}")
        }
    }
}

// ---------------------------------------------------------------------------
// Test 5 — expansion preserves provenance
// ---------------------------------------------------------------------------

#[test]
fn expansion_preserves_provenance() {
    let provenance = Provenance::Declared(vec![(
        "output_col".to_string(),
        vec!["input_col".to_string()],
    )]);
    let plan = make_transparent_call_with_provenance("prov_fn", provenance.clone());

    let rule = ExpandTransparentFunctionCalls;
    let ctx = RuleContext::default();
    let result = rule.apply(plan, &ctx);

    let new_plan = match result {
        RuleResult::Changed(p) => p,
        RuleResult::Unchanged => panic!("Expected Changed"),
    };

    // Unwrap Cast wrapper if present, then check ExpandedCall provenance.
    let expanded = match new_plan.as_ref() {
        LogicalNode::Cast { inner, .. } => inner.clone(),
        _ => new_plan.clone(),
    };

    match expanded.as_ref() {
        LogicalNode::ExpandedCall {
            provenance: stored_provenance,
            ..
        } => {
            assert_eq!(
                *stored_provenance, provenance,
                "ExpandedCall must carry original provenance; got: {stored_provenance:?}"
            );
        }
        other => panic!("Expected ExpandedCall, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 6 — Cast emitted for needs_cast returns
// ---------------------------------------------------------------------------

#[test]
fn cast_emitted_for_needs_cast_returns() {
    // Cast emission requires both `needs_cast: true` AND a registered canonical
    // return for the function. SUM has both, so the rule emits a Cast wrapper.
    let plan = make_needs_cast_call("SUM");
    let rule = ExpandTransparentFunctionCalls;
    let ctx = RuleContext::with_builtins();

    let result = rule.apply(plan, &ctx);
    let new_plan = match result {
        RuleResult::Changed(p) => p,
        RuleResult::Unchanged => panic!("Expected Changed"),
    };

    // Top level must be a Cast node wrapping an ExpandedCall.
    match new_plan.as_ref() {
        LogicalNode::Cast {
            inner,
            target_type: _,
        } => match inner.as_ref() {
            LogicalNode::ExpandedCall { fn_id, .. } => {
                assert_eq!(fn_id, "SUM", "ExpandedCall fn_id must be preserved");
            }
            other => panic!("Expected ExpandedCall inside Cast, got: {other:?}"),
        },
        other => panic!("Expected Cast node at top level for needs_cast=true, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Integration: apply_rules_to_fixed_point helper
// ---------------------------------------------------------------------------

#[test]
fn apply_rules_fixed_point_runs_to_completion() {
    let plan = make_transparent_call("fn_a");
    let rules: Vec<Box<dyn PlannerRule>> = vec![Box::new(ExpandTransparentFunctionCalls)];

    let result = apply_rules_to_fixed_point(plan, &rules);

    assert!(
        !has_transparent_call(&result),
        "Fixed-point loop must have removed all transparent calls; got: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Phase 40 — CAST emission resolves target type from Signature::canonical_return
// ---------------------------------------------------------------------------

#[test]
fn sum_integer_cast_target_is_bigint() {
    let plan = make_needs_cast_call("SUM");
    let rule = ExpandTransparentFunctionCalls;
    let ctx = RuleContext::with_builtins();

    let result = rule.apply(plan, &ctx);
    let new_plan = match result {
        RuleResult::Changed(p) => p,
        RuleResult::Unchanged => panic!("Expected Changed for needs_cast=true SUM call"),
    };

    match new_plan.as_ref() {
        LogicalNode::Cast { target_type, .. } => {
            assert_eq!(
                *target_type,
                DataType::BigInt,
                "SUM canonical return must resolve to BigInt; got {target_type:?}"
            );
        }
        other => panic!("Expected Cast wrapping ExpandedCall, got: {other:?}"),
    }
}

#[test]
fn sum_decimal_cast_target_is_decimal() {
    // Per §16 #9, Decimal precision/scale tracking is deferred for v1.
    // SUM's canonical_return is encoded once per signature; the v1 acceptance
    // for SUM(Decimal) is either BigInt (the registry's static canonical) or
    // Decimal { precision: 38, scale: 0 } (the per-engine native widening).
    let plan = make_needs_cast_call("SUM");
    let rule = ExpandTransparentFunctionCalls;
    let ctx = RuleContext::with_builtins();

    let result = rule.apply(plan, &ctx);
    let new_plan = match result {
        RuleResult::Changed(p) => p,
        RuleResult::Unchanged => panic!("Expected Changed for needs_cast=true SUM call"),
    };

    match new_plan.as_ref() {
        LogicalNode::Cast { target_type, .. } => {
            let acceptable = matches!(target_type, DataType::BigInt)
                || matches!(
                    target_type,
                    DataType::Decimal {
                        precision: 38,
                        scale: 0
                    }
                );
            assert!(
                acceptable,
                "SUM(Decimal) canonical must be BigInt or Decimal(38,0); got {target_type:?}"
            );
        }
        other => panic!("Expected Cast wrapping ExpandedCall, got: {other:?}"),
    }
}

#[test]
fn non_cast_function_emits_no_cast_node() {
    let plan = make_transparent_call("LOWER");
    let rule = ExpandTransparentFunctionCalls;
    let ctx = RuleContext::with_builtins();

    let result = rule.apply(plan, &ctx);
    let new_plan = match result {
        RuleResult::Changed(p) => p,
        RuleResult::Unchanged => panic!("Expected Changed for transparent LOWER call"),
    };

    match new_plan.as_ref() {
        LogicalNode::ExpandedCall { fn_id, .. } => {
            assert_eq!(fn_id, "LOWER");
        }
        LogicalNode::Cast { .. } => {
            panic!("Cast wrapper must not be emitted for needs_cast=false; got: {new_plan:?}")
        }
        other => panic!("Expected ExpandedCall, got: {other:?}"),
    }
}

#[test]
fn unknown_signature_falls_back_to_no_cast() {
    // Function name not in the registry, with needs_cast=true.  The lookup
    // returns None; the rule must omit the Cast wrapper rather than panic
    // or emit a Cast with an arbitrary placeholder target type.
    let plan = make_needs_cast_call("totally_unknown_extern_fn");
    let rule = ExpandTransparentFunctionCalls;
    let ctx = RuleContext::with_builtins();

    let result = rule.apply(plan, &ctx);
    let new_plan = match result {
        RuleResult::Changed(p) => p,
        RuleResult::Unchanged => panic!("Expected Changed for transparent call"),
    };

    match new_plan.as_ref() {
        LogicalNode::ExpandedCall { fn_id, .. } => {
            assert_eq!(fn_id, "totally_unknown_extern_fn");
        }
        LogicalNode::Cast { .. } => {
            panic!("Cast must not be emitted when canonical_return is unknown; got: {new_plan:?}")
        }
        other => panic!("Expected ExpandedCall (no Cast wrapper), got: {other:?}"),
    }
}
