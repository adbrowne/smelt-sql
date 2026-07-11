//! Phase MP16 (`docs/plans/20260707-maintenance-plan-impl.md`): the
//! adjointness law between the graph layer's two directions
//! (`maintenance_plan.md` §"Backward resolution — what must exist": "The two
//! directions are **adjoint, not inverse**: `forward(backward(P)) ⊇ P`").
//!
//! Pure math only — no CLI, no runtime, no on-disk workspace. Exercises
//! `propagate`/`required_inputs` directly over hand-typed [`Edge`] lists,
//! mirroring `maintenance_tracer_propagation.rs`'s style. That suite already
//! carries one adjointness test inline
//! (`forward_of_backward_covers_the_requested_period`) as part of its wider
//! composition-math regression floor; this file is MP16's own dedicated
//! home for the law, with a chain and a diamond shape so the containment
//! is pinned across more than one graph topology.

use std::collections::BTreeMap;

use smelt_logical::maintenance::propagate::{propagate, required_inputs, DayInterval, Edge};

fn iv(start: i64, end: i64) -> DayInterval {
    DayInterval::new(start, end)
}

fn edge(upstream: &str, downstream: &str, before_days: i64, after_days: i64) -> Edge {
    Edge {
        upstream: upstream.to_string(),
        downstream: downstream.to_string(),
        before_days,
        after_days,
        upstream_grain: Default::default(),
        downstream_grain: Default::default(),
    }
}

/// Resolve `required_inputs` for `target`/`period`, replay every raw
/// source's resolved slice as a forward delta, and assert the forward
/// result's dirt on `target` contains `period`.
fn assert_forward_backward_containment(edges: &[Edge], target: &str, period: DayInterval) {
    let resolved = required_inputs(edges, target, period).expect("resolve");

    // Raw sources are exactly the required nodes that never appear as a
    // downstream of any edge.
    let downstreams: std::collections::BTreeSet<&str> =
        edges.iter().map(|e| e.downstream.as_str()).collect();
    let mut replay: BTreeMap<String, Vec<DayInterval>> = BTreeMap::new();
    for (node, intervals) in &resolved.required {
        if !downstreams.contains(node.as_str()) {
            replay.insert(node.clone(), intervals.clone());
        }
    }

    let forward = propagate(edges, &replay).expect("propagate");
    let dirty = forward.dirty.get(target).unwrap_or_else(|| {
        panic!(
            "target '{target}' must be dirty after replaying its own resolved sources: {forward:?}"
        )
    });
    assert!(
        dirty
            .iter()
            .any(|d| d.start <= period.start && period.end <= d.end),
        "forward(backward({period:?})) must cover {period:?} for target '{target}'; got {dirty:?}"
    );
}

/// A straight three-hop chain: `bronze -> silver -> rollup`. Replaying
/// `required_inputs("rollup", period)`'s resolved `bronze` slice forward
/// must dirty `rollup` over at least `period`.
#[test]
fn forward_backward_containment_over_a_chain() {
    let edges = vec![
        edge("bronze", "silver", 0, 2),
        edge("conversions", "silver", 0, 14),
        edge("silver", "rollup", 1, 0),
    ];
    assert_forward_backward_containment(&edges, "rollup", iv(4, 7));
}

/// A diamond: `src` feeds both `a` and `b`, which both feed `sink`. The
/// backward resolution merges `src`'s requirement across both paths;
/// replaying that merged slice forward must still cover the requested
/// period at `sink`.
#[test]
fn forward_backward_containment_over_a_diamond() {
    let edges = vec![
        edge("src", "a", 0, 0),
        edge("src", "b", 1, 0),
        edge("a", "sink", 0, 0),
        edge("b", "sink", 3, 0),
    ];
    assert_forward_backward_containment(&edges, "sink", iv(10, 11));
}

/// A target with NO inbound edge in the graph (e.g. a `refresh: full` model,
/// or any target the propagation graph never registered an upstream edge
/// for) must still appear in `build_order`, as the last entry — "no upstream
/// deps" means an empty required-slices set for ancestors, never an empty
/// build for the target itself. Regression for the bug where `build_order`'s
/// `has_inbound` filter silently dropped such a target, which
/// `resolve_build_plan`/`build_include_upstreams` then read as "nothing to
/// build" and skipped the model entirely.
#[test]
fn build_order_always_includes_a_target_with_no_inbound_edge() {
    // `standalone` never appears as a `downstream` of any edge — these edges
    // exist only so the graph is non-empty; `standalone` is untouched by any
    // of them.
    let edges = vec![
        edge("bronze", "silver", 0, 2),
        edge("silver", "rollup", 1, 0),
    ];
    let resolved = required_inputs(&edges, "standalone", iv(4, 7)).expect("resolve");
    assert_eq!(
        resolved.build_order,
        vec!["standalone".to_string()],
        "a target with no inbound edge must still be the (only, last) entry in build_order: \
         {resolved:?}"
    );
    // Its own required slice is exactly the requested period — no ancestors
    // are pulled in, since there is no inbound edge to walk.
    assert_eq!(
        resolved.required.get("standalone"),
        Some(&vec![iv(4, 7)]),
        "{resolved:?}"
    );
}

/// The same no-inbound-edge target, but *embedded* alongside an unrelated
/// chain in the same edge list — proves the fix doesn't just handle the
/// trivially-empty-graph case, and that only the target (not unrelated
/// nodes) gets the "always include" carve-out.
#[test]
fn build_order_includes_no_inbound_target_alongside_an_unrelated_chain() {
    let edges = vec![
        edge("bronze", "silver", 0, 2),
        edge("silver", "rollup", 1, 0),
    ];
    let resolved = required_inputs(&edges, "standalone", iv(0, 1)).expect("resolve");
    assert_eq!(resolved.build_order, vec!["standalone".to_string()]);
    // The unrelated chain's nodes are not pulled into `required` at all —
    // `standalone` shares no edge with them.
    assert!(!resolved.required.contains_key("bronze"));
    assert!(!resolved.required.contains_key("silver"));
    assert!(!resolved.required.contains_key("rollup"));
}
