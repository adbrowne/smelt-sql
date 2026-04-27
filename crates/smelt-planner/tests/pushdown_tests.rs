//! Phase 33 — `PushFilterIntoTransparentFunction` rule tests.
//!
//! These tests verify that WHERE predicates from a `Select` node are pushed
//! into a transparent `FunctionCall` (via `pushed_filter`), and that the rule
//! is correctly blocked for opaque provenance and non-transparent calls.

use std::sync::Arc;

use smelt_planner::logical::{FunctionProperties, LogicalNode, Provenance};
use smelt_planner::logical_plan_rules::{
    apply_rules_to_fixed_point, ExpandTransparentFunctionCalls, PlannerRule,
    PushFilterIntoTransparentFunction, RuleContext, RuleResult,
};
use smelt_types::DataType;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn declared_provenance() -> Provenance {
    Provenance::Declared(vec![(
        "margin".to_string(),
        vec!["source.revenue".to_string()],
    )])
}

fn make_transparent_call_deterministic(fn_id: &str, provenance: Provenance) -> Arc<LogicalNode> {
    Arc::new(LogicalNode::FunctionCall {
        fn_id: fn_id.to_string(),
        args: vec![],
        transparent: true,
        provenance,
        properties: FunctionProperties {
            deterministic: true,
            ..FunctionProperties::default()
        },
        pushed_filter: None,
        body: None,
    })
}

fn make_opaque_call_deterministic(fn_id: &str, provenance: Provenance) -> Arc<LogicalNode> {
    Arc::new(LogicalNode::FunctionCall {
        fn_id: fn_id.to_string(),
        args: vec![],
        transparent: false,
        provenance,
        properties: FunctionProperties {
            deterministic: true,
            ..FunctionProperties::default()
        },
        pushed_filter: None,
        body: None,
    })
}

fn boolean_literal() -> Arc<LogicalNode> {
    Arc::new(LogicalNode::Literal(DataType::Boolean))
}

fn make_select_with_filter(from: Arc<LogicalNode>, filter: Arc<LogicalNode>) -> Arc<LogicalNode> {
    Arc::new(LogicalNode::Select {
        projections: vec!["*".to_string()],
        from: Some(from),
        filter: Some(filter),
    })
}

// ---------------------------------------------------------------------------
// Test 1 — push filter into transparent function body
// ---------------------------------------------------------------------------
//
// A Select{from: FunctionCall{transparent:true, deterministic:true, provenance:Declared},
//           filter: Literal(Boolean)}
// must be rewritten so that:
//   - the FunctionCall gains pushed_filter = Some(Literal(Boolean))
//   - the enclosing Select's filter is cleared (None)

#[test]
fn pushdown_into_transparent_function_body() {
    let call = make_transparent_call_deterministic("add_margin", declared_provenance());
    let pred = boolean_literal();
    let plan = make_select_with_filter(call, pred.clone());

    let rule = PushFilterIntoTransparentFunction;
    let result = rule.apply(plan, &RuleContext::default());

    let new_plan = match result {
        RuleResult::Changed(p) => p,
        RuleResult::Unchanged => panic!("Expected Changed: rule should push filter"),
    };

    // The outer Select's filter must be cleared.
    match new_plan.as_ref() {
        LogicalNode::Select { filter, from, .. } => {
            assert!(
                filter.is_none(),
                "Select filter must be cleared after pushdown; got: {filter:?}"
            );

            // The from child must be a FunctionCall with pushed_filter set.
            let from_node = from.as_ref().expect("Select must have a from child");
            match from_node.as_ref() {
                LogicalNode::FunctionCall { pushed_filter, .. } => {
                    assert!(
                        pushed_filter.is_some(),
                        "FunctionCall must have pushed_filter set after pushdown"
                    );
                    assert_eq!(
                        pushed_filter.as_ref().unwrap().as_ref(),
                        pred.as_ref(),
                        "pushed_filter must equal the original predicate"
                    );
                }
                other => panic!("Expected FunctionCall as from child, got: {other:?}"),
            }
        }
        other => panic!("Expected Select at top level, got: {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Test 2 — pushdown blocked by opaque provenance
// ---------------------------------------------------------------------------
//
// When the FunctionCall has provenance = Unknown, the rule must not push.

#[test]
fn pushdown_blocked_by_opaque_provenance() {
    let call = make_transparent_call_deterministic("add_margin", Provenance::Unknown);
    let pred = boolean_literal();
    let plan = make_select_with_filter(call, pred);

    let rule = PushFilterIntoTransparentFunction;
    match rule.apply(plan, &RuleContext::default()) {
        RuleResult::Unchanged => {}
        RuleResult::Changed(p) => {
            panic!("Expected Unchanged for opaque provenance, got Changed: {p:?}")
        }
    }
}

// ---------------------------------------------------------------------------
// Test 3 — pushdown blocked at black-box (non-transparent) call
// ---------------------------------------------------------------------------
//
// When the FunctionCall has transparent = false, the rule must not push.

#[test]
fn pushdown_blocked_at_black_box() {
    let call = make_opaque_call_deterministic("ext_fn", declared_provenance());
    let pred = boolean_literal();
    let plan = make_select_with_filter(call, pred);

    let rule = PushFilterIntoTransparentFunction;
    match rule.apply(plan, &RuleContext::default()) {
        RuleResult::Unchanged => {}
        RuleResult::Changed(p) => {
            panic!("Expected Unchanged for non-transparent call, got Changed: {p:?}")
        }
    }
}

// ---------------------------------------------------------------------------
// Test 4 — combined rule set reaches fixed point with correct ordering
// ---------------------------------------------------------------------------
//
// Filter pushdown runs BEFORE expansion so predicates are captured before
// the transparent FunctionCall is replaced by an ExpandedCall. Ordering:
// [PushFilterIntoTransparentFunction, ExpandTransparentFunctionCalls].
//
// Assertions:
//   a) The outer Select's filter is None after the run (predicate consumed).
//   b) No transparent FunctionCall nodes remain (expansion ran).
//   c) A second run produces an identical plan (fixed point).

#[test]
fn combined_rule_set_reaches_fixed_point() {
    let call = make_transparent_call_deterministic("add_margin", declared_provenance());
    let pred = boolean_literal();
    let plan = make_select_with_filter(call, pred);

    // Pushdown runs first so the filter is captured before the FunctionCall is expanded.
    let rules: Vec<Box<dyn PlannerRule>> = vec![
        Box::new(PushFilterIntoTransparentFunction),
        Box::new(ExpandTransparentFunctionCalls),
    ];

    let result = apply_rules_to_fixed_point(plan, &rules);

    // (a) The outer Select's filter must be cleared.
    match result.as_ref() {
        LogicalNode::Select { filter, .. } => {
            assert!(
                filter.is_none(),
                "filter must be consumed after combined pass"
            );
        }
        other => panic!("Expected Select at root, got {other:?}"),
    }

    // (b) No transparent FunctionCall must remain.
    assert!(
        !contains_transparent_function_call(&result),
        "no transparent FunctionCall should remain after expansion"
    );

    // (c) Fixed point — second pass yields identical plan.
    let result2 = apply_rules_to_fixed_point(result.clone(), &rules);
    assert_eq!(
        result, result2,
        "combined rule set must be stable on second pass"
    );
}

fn contains_transparent_function_call(node: &Arc<LogicalNode>) -> bool {
    match node.as_ref() {
        LogicalNode::FunctionCall {
            transparent: true, ..
        } => true,
        LogicalNode::FunctionCall {
            transparent: false, ..
        } => false,
        LogicalNode::Select { from, filter, .. } => {
            from.as_ref()
                .is_some_and(contains_transparent_function_call)
                || filter
                    .as_ref()
                    .is_some_and(contains_transparent_function_call)
        }
        LogicalNode::ExpandedCall { body, .. } => body
            .as_ref()
            .is_some_and(contains_transparent_function_call),
        LogicalNode::TableRef { .. } | LogicalNode::Literal(_) | LogicalNode::Raw { .. } => false,
        LogicalNode::Cast { inner, .. } => contains_transparent_function_call(inner),
        LogicalNode::Tagged { inner, .. } => contains_transparent_function_call(inner),
        LogicalNode::SpliceList(items) => items.iter().any(contains_transparent_function_call),
        LogicalNode::LeftJoin { lhs, rhs, .. } => {
            contains_transparent_function_call(lhs) || contains_transparent_function_call(rhs)
        }
    }
}

// ---------------------------------------------------------------------------
// Test 6 — pushdown blocked by non-deterministic function
// ---------------------------------------------------------------------------
//
// When properties.deterministic == false the rule must return Unchanged even
// if transparent == true and provenance is Declared.

#[test]
fn pushdown_blocked_by_non_deterministic_function() {
    let call = Arc::new(LogicalNode::FunctionCall {
        fn_id: "random_fn".to_string(),
        args: vec![],
        transparent: true,
        provenance: declared_provenance(),
        properties: FunctionProperties {
            deterministic: false, // <-- not deterministic
            ..FunctionProperties::default()
        },
        pushed_filter: None,
        body: None,
    });
    let pred = boolean_literal();
    let plan = make_select_with_filter(call, pred);

    let rule = PushFilterIntoTransparentFunction;
    match rule.apply(plan, &RuleContext::default()) {
        RuleResult::Unchanged => {}
        RuleResult::Changed(p) => {
            panic!("Expected Unchanged for non-deterministic function, got Changed: {p:?}")
        }
    }
}

// ---------------------------------------------------------------------------
// Test 5 — no re-push when pushed_filter already set
// ---------------------------------------------------------------------------
//
// If a FunctionCall already has pushed_filter set, the rule must skip it
// (idempotent), even if the outer Select has no filter to push (which would
// be the case after a prior pass).

#[test]
fn no_re_push_when_already_pushed() {
    // Build a plan where the FunctionCall already has pushed_filter set and
    // the Select has no filter (state after a successful push pass).
    let pred = boolean_literal();
    let call_already_pushed = Arc::new(LogicalNode::FunctionCall {
        fn_id: "add_margin".to_string(),
        args: vec![],
        transparent: true,
        provenance: declared_provenance(),
        properties: FunctionProperties {
            deterministic: true,
            ..FunctionProperties::default()
        },
        pushed_filter: Some(pred),
        body: None,
    });
    let plan = Arc::new(LogicalNode::Select {
        projections: vec!["*".to_string()],
        from: Some(call_already_pushed),
        filter: None, // already cleared
    });

    let rule = PushFilterIntoTransparentFunction;
    match rule.apply(plan, &RuleContext::default()) {
        RuleResult::Unchanged => {}
        RuleResult::Changed(p) => {
            panic!("Expected Unchanged when pushed_filter already set, got Changed: {p:?}")
        }
    }
}
