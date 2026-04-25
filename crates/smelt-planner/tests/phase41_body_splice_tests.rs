//! Phase 41 — body-splice + list-splice comma elision tests.
//!
//! These pure-Rust tests build `LogicalNode` trees directly and drive the
//! Phase 41 expansion + elision rules through the public API in
//! `smelt_planner::logical_plan_rules`.  They do not depend on Salsa or on
//! the `smelt-db::logical_plan` query — Salsa-side cycle detection is
//! exercised separately under `crates/smelt-cli/tests/`.

use std::sync::Arc;

use smelt_planner::logical::{FunctionProperties, LogicalNode, Plan, Provenance, ProvenanceTag};
use smelt_planner::logical_plan_rules::{
    apply_rules_to_fixed_point, ElideEmptySelectItemsSplices, ExpandTransparentFunctionCalls,
    PlannerRule, RuleContext, RuleResult,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_transparent_call_with_body(fn_id: &str, body: Plan) -> Plan {
    Arc::new(LogicalNode::FunctionCall {
        fn_id: fn_id.to_string(),
        args: vec![],
        transparent: true,
        provenance: Provenance::Unknown,
        properties: FunctionProperties::default(),
        pushed_filter: None,
        body: Some(body),
    })
}

fn make_transparent_call_no_body(fn_id: &str) -> Plan {
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

fn raw(text: &str) -> Plan {
    Arc::new(LogicalNode::Raw {
        sql_text: text.to_string(),
    })
}

/// Recursively check whether any node in `plan` has the given provenance tag.
fn contains_tag(plan: &Plan, predicate: impl Fn(&ProvenanceTag) -> bool + Copy) -> bool {
    match plan.as_ref() {
        LogicalNode::Tagged { tag, inner } => predicate(tag) || contains_tag(inner, predicate),
        LogicalNode::Cast { inner, .. } => contains_tag(inner, predicate),
        LogicalNode::Select { from, filter, .. } => {
            from.as_ref().is_some_and(|p| contains_tag(p, predicate))
                || filter.as_ref().is_some_and(|p| contains_tag(p, predicate))
        }
        LogicalNode::ExpandedCall { body, .. } => {
            body.as_ref().is_some_and(|p| contains_tag(p, predicate))
        }
        LogicalNode::FunctionCall { args, body, .. } => {
            args.iter().any(|a| contains_tag(a, predicate))
                || body.as_ref().is_some_and(|b| contains_tag(b, predicate))
        }
        LogicalNode::SpliceList(items) => items.iter().any(|p| contains_tag(p, predicate)),
        LogicalNode::LeftJoin { lhs, rhs, .. } => {
            contains_tag(lhs, predicate) || contains_tag(rhs, predicate)
        }
        LogicalNode::TableRef { .. } | LogicalNode::Literal(_) | LogicalNode::Raw { .. } => false,
    }
}

/// Find the first `ExpandedCall` for `fn_id` in `plan`.  Recurses through
/// `Cast`, `Tagged`, `Select`, `LeftJoin`, and into `ExpandedCall.body`.
fn find_expanded_call<'a>(plan: &'a Plan, fn_id: &str) -> Option<&'a LogicalNode> {
    fn walk<'a>(plan: &'a Plan, fn_id: &str) -> Option<&'a LogicalNode> {
        match plan.as_ref() {
            LogicalNode::ExpandedCall {
                fn_id: id, body, ..
            } => {
                if id == fn_id {
                    Some(plan.as_ref())
                } else if let Some(b) = body {
                    walk(b, fn_id)
                } else {
                    None
                }
            }
            LogicalNode::Cast { inner, .. } | LogicalNode::Tagged { inner, .. } => {
                walk(inner, fn_id)
            }
            LogicalNode::Select { from, filter, .. } => from
                .as_ref()
                .and_then(|p| walk(p, fn_id))
                .or_else(|| filter.as_ref().and_then(|p| walk(p, fn_id))),
            LogicalNode::SpliceList(items) => items.iter().find_map(|p| walk(p, fn_id)),
            LogicalNode::LeftJoin { lhs, rhs, .. } => walk(lhs, fn_id).or_else(|| walk(rhs, fn_id)),
            _ => None,
        }
    }
    walk(plan, fn_id)
}

// ---------------------------------------------------------------------------
// Test 1 — expanded_call_contains_body_subtree
// ---------------------------------------------------------------------------

#[test]
fn expanded_call_contains_body_subtree() {
    let body = raw("SELECT 1 AS v");
    let plan = make_transparent_call_with_body("identity_fn", body.clone());

    let result = apply_rules_to_fixed_point(plan, &[Box::new(ExpandTransparentFunctionCalls)]);

    let expanded = find_expanded_call(&result, "identity_fn")
        .expect("expected an ExpandedCall for identity_fn");
    let LogicalNode::ExpandedCall { body: spliced, .. } = expanded else {
        panic!("find_expanded_call returned a non-ExpandedCall node: {expanded:?}");
    };
    let spliced = spliced
        .as_ref()
        .expect("ExpandedCall.body must be Some(_) when the FunctionCall carried a body");

    // The spliced body must surface the original body's content.  We assert
    // structurally by walking through the provenance tag wrapper to find the
    // Raw node.
    fn find_raw(p: &Plan) -> Option<String> {
        match p.as_ref() {
            LogicalNode::Raw { sql_text } => Some(sql_text.clone()),
            LogicalNode::Tagged { inner, .. } => find_raw(inner),
            LogicalNode::Cast { inner, .. } => find_raw(inner),
            LogicalNode::Select { from, .. } => from.as_ref().and_then(find_raw),
            _ => None,
        }
    }
    let raw_text = find_raw(spliced).expect("spliced body must contain the original Raw subtree");
    assert_eq!(raw_text, "SELECT 1 AS v");
}

// ---------------------------------------------------------------------------
// Test 2 — nested_transparent_calls_expand_recursively
// ---------------------------------------------------------------------------

#[test]
fn nested_transparent_calls_expand_recursively() {
    // A's body calls B; B's body calls C; C's body is a Raw subtree.
    let c_body = raw("SELECT 'c'");
    let c_call = make_transparent_call_with_body("c", c_body);

    let b_call = make_transparent_call_with_body("b", c_call);
    let a_call = make_transparent_call_with_body("a", b_call);

    let result = apply_rules_to_fixed_point(a_call, &[Box::new(ExpandTransparentFunctionCalls)]);

    // The outer plan must be an ExpandedCall for `a` whose body resolves
    // (eventually) to an ExpandedCall for `c`.
    let outer = match result.as_ref() {
        LogicalNode::ExpandedCall { fn_id, .. } if fn_id == "a" => result.clone(),
        _ => panic!("expected outer ExpandedCall for a, got: {result:?}"),
    };
    let _b = find_expanded_call(&outer, "b").expect("expected nested ExpandedCall for b");
    let _c = find_expanded_call(&outer, "c").expect("expected nested ExpandedCall for c");
}

// ---------------------------------------------------------------------------
// Test 3 — cycle_detection_terminates_expansion
// ---------------------------------------------------------------------------
//
// A synthesised cycle is exercised here at the planner-rule level: when the
// plan builder leaves `body: None` (the smelt-db cycle pre-pass's signal), the
// expansion rule must fall back to the marker behaviour and the fixed-point
// loop must terminate.  The Salsa-side `FunctionCallCycle` diagnostic is
// asserted in the broken-fixture harness under crates/smelt-cli/tests/.

#[test]
fn cycle_detection_terminates_expansion() {
    // Synthesise a call without a body — the way the smelt-db cycle pre-pass
    // signals to the planner that expansion is unsafe.
    let plan = make_transparent_call_no_body("a");
    let result =
        apply_rules_to_fixed_point(plan.clone(), &[Box::new(ExpandTransparentFunctionCalls)]);

    // The fixed-point loop terminated (we are here, no infinite recursion).
    // The result is an ExpandedCall with body=None — the Phase 32 marker
    // behaviour preserved by Phase 41 for cycle-suppressed calls.
    let LogicalNode::ExpandedCall { fn_id, body, .. } = result.as_ref() else {
        panic!("expected ExpandedCall fall-back, got: {result:?}");
    };
    assert_eq!(fn_id, "a");
    assert!(body.is_none(), "cycle-suppressed body must remain None");
}

// ---------------------------------------------------------------------------
// Test 4 — empty_selectitems_default_elides_comma
// ---------------------------------------------------------------------------
//
// `SELECT user_col, ..., metrics FROM sessionized` with `metrics = ()` lowers
// (at the plan level) to a SpliceList that drops the empty splice point.  We
// assert this by feeding a SpliceList with an empty inner SpliceList to
// `ElideEmptySelectItemsSplices`; the inner empty splice must vanish.

#[test]
fn empty_selectitems_default_elides_comma() {
    let user_col = raw("user_col");
    let other = raw("other");
    let metrics_empty: Plan = Arc::new(LogicalNode::SpliceList(vec![]));

    let projections: Plan = Arc::new(LogicalNode::SpliceList(vec![
        user_col.clone(),
        other.clone(),
        metrics_empty,
    ]));

    let result = apply_rules_to_fixed_point(projections, &[Box::new(ElideEmptySelectItemsSplices)]);

    let LogicalNode::SpliceList(items) = result.as_ref() else {
        panic!("expected SpliceList at root, got: {result:?}");
    };
    assert_eq!(
        items.len(),
        2,
        "empty splice must be elided; got {items:#?}"
    );
    // No nested SpliceList must remain.
    for it in items {
        assert!(
            !matches!(it.as_ref(), LogicalNode::SpliceList(_)),
            "no nested SpliceList must remain after elision: {it:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Test 5 — non_empty_selectitems_keeps_commas
// ---------------------------------------------------------------------------
//
// `metrics = (COUNT(*) AS n)` lowers with the trailing column included.  At
// the plan level, a SpliceList wrapping `[count_star]` flattens into the
// parent context: the parent ends up with `[user_col, other, count_star]`.

#[test]
fn non_empty_selectitems_keeps_commas() {
    let user_col = raw("user_col");
    let other = raw("other");
    let count_star = raw("COUNT(*) AS n");
    let metrics: Plan = Arc::new(LogicalNode::SpliceList(vec![count_star.clone()]));

    let projections: Plan = Arc::new(LogicalNode::SpliceList(vec![
        user_col.clone(),
        other.clone(),
        metrics,
    ]));

    let result = apply_rules_to_fixed_point(
        projections.clone(),
        &[Box::new(ElideEmptySelectItemsSplices)],
    );

    let LogicalNode::SpliceList(items) = result.as_ref() else {
        panic!("expected SpliceList at root, got: {result:?}");
    };
    assert_eq!(items.len(), 3, "non-empty splice contributes its items");
    let extracted: Vec<_> = items
        .iter()
        .map(|p| match p.as_ref() {
            LogicalNode::Raw { sql_text } => sql_text.clone(),
            other => panic!("expected only Raw children, got {other:?}"),
        })
        .collect();
    assert_eq!(extracted, vec!["user_col", "other", "COUNT(*) AS n"]);
}

// ---------------------------------------------------------------------------
// Test 6 — provenance_preserved_through_splice
// ---------------------------------------------------------------------------

#[test]
fn provenance_preserved_through_splice() {
    let body = raw("SELECT 1");
    let plan = make_transparent_call_with_body("foo", body);

    let result = apply_rules_to_fixed_point(plan, &[Box::new(ExpandTransparentFunctionCalls)]);

    // Every spliced node carries a `Callee("foo")` tag — at minimum the
    // outermost wrapper does.
    let expanded = find_expanded_call(&result, "foo").expect("ExpandedCall for foo must exist");
    let LogicalNode::ExpandedCall { body, .. } = expanded else {
        unreachable!()
    };
    let body = body.as_ref().expect("spliced body must be present");
    assert!(
        contains_tag(
            body,
            |t| matches!(t, ProvenanceTag::Callee(id) if id == "foo")
        ),
        "expected at least one Callee(\"foo\") tag in the spliced body; got: {body:?}"
    );
}

// ---------------------------------------------------------------------------
// Idempotency — running ElideEmptySelectItemsSplices twice == once
// ---------------------------------------------------------------------------
//
// Phase 41's review checklist: comma-elision rule is idempotent.

#[test]
fn elide_empty_splices_is_idempotent() {
    let projections: Plan = Arc::new(LogicalNode::SpliceList(vec![
        raw("a"),
        Arc::new(LogicalNode::SpliceList(vec![])),
        raw("b"),
        Arc::new(LogicalNode::SpliceList(vec![raw("c")])),
    ]));

    let once = apply_rules_to_fixed_point(
        projections.clone(),
        &[Box::new(ElideEmptySelectItemsSplices)],
    );
    let twice = apply_rules_to_fixed_point(once.clone(), &[Box::new(ElideEmptySelectItemsSplices)]);

    let LogicalNode::SpliceList(items_once) = once.as_ref() else {
        panic!("once: expected SpliceList");
    };
    let LogicalNode::SpliceList(items_twice) = twice.as_ref() else {
        panic!("twice: expected SpliceList");
    };

    assert_eq!(items_once.len(), 3, "once should flatten to [a, b, c]");
    assert_eq!(items_twice.len(), 3, "twice should match once");
    assert_eq!(once, twice, "rule is idempotent");
}

// ---------------------------------------------------------------------------
// Marker behaviour preserved when body is absent
// ---------------------------------------------------------------------------
//
// Defensive: when `FunctionCall.body` is `None`, the expansion rule must
// continue to emit an `ExpandedCall` marker (Phase 32 behaviour).  This is
// the path used by opaque `smelt.extern` calls and by cycle-suppressed
// transparent calls.

#[test]
fn marker_behaviour_when_body_absent() {
    let plan = make_transparent_call_no_body("foo");
    let result = apply_rules_to_fixed_point(plan, &[Box::new(ExpandTransparentFunctionCalls)]);

    let LogicalNode::ExpandedCall { fn_id, body, .. } = result.as_ref() else {
        panic!("expected ExpandedCall, got: {result:?}");
    };
    assert_eq!(fn_id, "foo");
    assert!(body.is_none());
}

// ---------------------------------------------------------------------------
// Visited-set cycle guard — synthesised self-referential body
// ---------------------------------------------------------------------------
//
// Defends the planner-rule against synthesised plans that escape the
// smelt-db cycle pre-pass: a transparent `FunctionCall` whose body contains
// another transparent `FunctionCall` with the same `fn_id`.  Without the
// per-pass visited-set keyed on `FnId`, `build_expanded_call` would recurse
// infinitely through `splice_body` → `expand_recursive` → `build_expanded_call`.

#[test]
fn synthesised_self_referential_body_terminates() {
    // Inner call shares the outer's fn_id and itself carries a body that
    // would re-enter the same fn_id if expansion were unguarded.
    let inner = make_transparent_call_with_body("loop", raw("SELECT 1"));
    let plan = make_transparent_call_with_body("loop", inner);

    let result = apply_rules_to_fixed_point(plan, &[Box::new(ExpandTransparentFunctionCalls)]);

    // Outer must be an ExpandedCall for `loop` whose spliced body contains
    // a marker (body=None) for the recursive inner call — the visited-set
    // suppresses the inner splice as if the smelt-db cycle pre-pass had
    // cleared its body.
    let LogicalNode::ExpandedCall {
        fn_id: outer_id,
        body: outer_body,
        ..
    } = result.as_ref()
    else {
        panic!("expected outer ExpandedCall, got: {result:?}");
    };
    assert_eq!(outer_id, "loop");
    let outer_body = outer_body
        .as_ref()
        .expect("outer ExpandedCall.body must be present (Phase 41 splice)");

    // Locate the inner ExpandedCall for the same fn_id.  Because the outer
    // entry is the first occurrence on the visited stack, find_expanded_call
    // walks past it via the body and surfaces the recursive inner one.
    fn find_inner_marker(p: &Plan) -> Option<&LogicalNode> {
        match p.as_ref() {
            LogicalNode::ExpandedCall {
                fn_id, body: None, ..
            } if fn_id == "loop" => Some(p.as_ref()),
            LogicalNode::Tagged { inner, .. } => find_inner_marker(inner),
            LogicalNode::Cast { inner, .. } => find_inner_marker(inner),
            LogicalNode::Select { from, filter, .. } => from
                .as_ref()
                .and_then(find_inner_marker)
                .or_else(|| filter.as_ref().and_then(find_inner_marker)),
            LogicalNode::SpliceList(items) => items.iter().find_map(find_inner_marker),
            LogicalNode::ExpandedCall {
                body: Some(b),
                fn_id,
                ..
            } if fn_id != "loop" => find_inner_marker(b),
            _ => None,
        }
    }
    assert!(
        find_inner_marker(outer_body).is_some(),
        "expected the inner recursive ExpandedCall for `loop` to fall back to body=None; got: {outer_body:?}"
    );
}

// Apply the variable to silence dead-code warnings if someone strips the
// helper unused — keeps the helper exercised in CI.
#[test]
fn _ensure_test_helpers_compile() {
    let p = make_transparent_call_no_body("x");
    let _ = find_expanded_call(&p, "y");
    let _ = contains_tag(&p, |_| false);
    let _ = RuleContext::default();
    let _ = RuleResult::Unchanged;
    let _ = ExpandTransparentFunctionCalls;
    let _ = ElideEmptySelectItemsSplices;
    let _: Box<dyn PlannerRule> = Box::new(ExpandTransparentFunctionCalls);
}
