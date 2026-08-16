//! Phase MP16 (`docs/plans/20260707-maintenance-plan-impl.md`): the
//! adjointness law between the graph layer's two directions
//! (`incremental_models.md` §"Backward resolution — what must exist": "The two
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

use smelt_logical::analysis::output_delta::OutputDelta;
use smelt_logical::maintenance::edge_type::{Addressing, EdgeComponent};
use smelt_logical::maintenance::locality::LocalitySlice;
use smelt_logical::maintenance::propagate::{
    project_observed_delta, propagate, required_inputs, DayInterval, Edge, KeyedDirt,
    PartitionGrain,
};

fn iv(start: i64, end: i64) -> DayInterval {
    DayInterval::new(start, end)
}

fn deltas(items: &[(&str, DayInterval)]) -> BTreeMap<String, Vec<DayInterval>> {
    let mut m: BTreeMap<String, Vec<DayInterval>> = BTreeMap::new();
    for (name, interval) in items {
        m.entry(name.to_string()).or_default().push(*interval);
    }
    m
}

fn edge(upstream: &str, downstream: &str, before_days: i64, after_days: i64) -> Edge {
    Edge {
        upstream: upstream.to_string(),
        downstream: downstream.to_string(),
        before_days,
        after_days,
        upstream_grain: Default::default(),
        downstream_grain: Default::default(),
        components: Vec::new(),
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

// ---------------------------------------------------------------------------
// Phase B1 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`):
// the graph layer admits a locality-admitted composed node (`grain: key` +
// `timeseries:`, admitted key temporal locality) as a clocked propagation
// participant at its own declared granularity — it is no longer refused as
// `PartitionGrain::Keyed`. A bare keyed node (no admitted time axis) still
// refuses, with a refined message naming the missing time axis and the
// composed-shape fix.
// ---------------------------------------------------------------------------

/// A chain `source -> composed(keyed+timeseries) -> rollup`: the middle
/// node stands in for a locality-admitted composed model, sandwiched
/// between two ordinary Day-grain nodes with its OWN declared granularity
/// (Month) — so the containment law also pins that the composed node's own
/// grain (not just its neighbours') drives outward alignment through it,
/// exactly as any other clocked node's would (`incremental_models.md`
/// §"The graph layer": "A locality-admitted time-partitioned keyed output
/// is not refused... a clocked node whose edges use its declared
/// granularity like any other node").
#[test]
fn composed_node_contributes_edges() {
    let mut into_composed = edge("source", "composed", 0, 0);
    into_composed.downstream_grain = PartitionGrain::Month;
    let mut out_of_composed = edge("composed", "rollup", 0, 0);
    out_of_composed.upstream_grain = PartitionGrain::Month;
    let edges = vec![into_composed, out_of_composed];
    assert_forward_backward_containment(&edges, "rollup", iv(400, 402));
}

/// A graph with no `PartitionGrain::Keyed` edge at all — precisely what a
/// locality-admitted composed node classifies as (its declared
/// granularity, never `Keyed`) — never trips `refuse_keyed_nodes`, in
/// either direction.
#[test]
fn admitted_composed_node_is_not_refused() {
    let edges = vec![
        edge("source", "composed", 0, 0),
        edge("composed", "rollup", 0, 0),
    ];
    propagate(&edges, &deltas(&[("source", iv(1, 2))])).expect("admitted composed node runs");
    required_inputs(&edges, "rollup", iv(1, 2)).expect("admitted composed node resolves");
}

/// Two SUCCESSIVE locality-admitted composed nodes in the same chain —
/// `source -> composed1 -> composed2 -> rollup` — the recursive shape
/// `docs/plans/20260719-prod-w8-composed-axes-followups.md` Phase 6 closes
/// at the graph-assembly layer (`smelt_runtime::propagation::
/// build_forward_graph` resolving `composed2`'s driving-source granularity
/// through `composed1`'s own admitted composed output, not only a declared
/// `sources.*` ref). This test pins the pure composition math those two
/// edges are fed into once assembled: neither `propagate` nor
/// `required_inputs` treats a `PartitionGrain::Keyed`-never (both hops
/// classify by their own declared granularity, mirroring
/// `composed_node_contributes_edges`) two-hop composed run any differently
/// from the single-hop case — the adjointness law
/// `forward(backward(P)) ⊇ P` still holds through both hops.
#[test]
fn two_composed_stages_in_a_chain_satisfy_adjointness() {
    let edges = vec![
        edge("source", "composed1", 0, 0),
        edge("composed1", "composed2", 0, 0),
        edge("composed2", "rollup", 0, 0),
    ];
    propagate(&edges, &deltas(&[("source", iv(1, 2))]))
        .expect("two successive admitted composed nodes must run without refusing");
    required_inputs(&edges, "rollup", iv(1, 2))
        .expect("two successive admitted composed nodes must resolve without refusing");
    assert_forward_backward_containment(&edges, "rollup", iv(100, 103));
}

/// The same two-composed-stage chain, but the first hop's inbound edge
/// carries a nonzero route-3-style margin (mirroring
/// `composed_projection_adjoint`'s route parameterisation) — the widened
/// projection from the FIRST composed stage must still compose correctly
/// through the SECOND composed stage's own (exact, zero-margin) edge, and
/// the adjointness law must hold over the full three-hop chain.
#[test]
fn two_composed_stages_adjoint_with_a_widened_first_hop() {
    let edges = vec![
        edge("source", "composed1", 3, 1),
        edge("composed1", "composed2", 0, 0),
        edge("composed2", "rollup", 0, 0),
    ];
    assert_forward_backward_containment(&edges, "rollup", iv(100, 103));
}

// ---------------------------------------------------------------------------
// Phase B2 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`):
// the composed node's own inbound edge carries a REAL route-derived margin
// (`smelt_logical::maintenance::propagate::locality_margin_days`) rather
// than B1's placeholder-exact zero — routes 1–2 project exactly, route 3
// widens backward by `r` + margins. `composed_projection_adjoint` pins the
// adjointness law over both shapes; the containment law extends to a strict
// superset (not equality) for the widened case.
// ---------------------------------------------------------------------------

/// The adjointness law `forward(backward(P)) ⊇ P` over a composed node,
/// route-parameterised: an exact (route 1/2, zero-margin) inbound edge and a
/// widened (route-3-style, nonzero-margin) inbound edge both satisfy the
/// law, through the same `source -> composed -> rollup` chain shape.
#[test]
fn composed_projection_adjoint() {
    for (before_days, after_days) in [(0, 0), (3, 1)] {
        let into_composed = edge("source", "composed", before_days, after_days);
        let out_of_composed = edge("composed", "rollup", 0, 0);
        let edges = vec![into_composed, out_of_composed];
        assert_forward_backward_containment(&edges, "rollup", iv(100, 103));
    }
}

/// Route 1/2 (exact, zero margin): the composed node's forward-projected
/// dirt from a replayed upstream delta equals the delta exactly — no
/// widening at all through the composed edge.
#[test]
fn composed_projection_exact_route_has_no_widening() {
    let edges = vec![edge("source", "composed", 0, 0)];
    let delta = iv(10, 12);
    let forward = propagate(&edges, &deltas(&[("source", delta)])).expect("propagate");
    assert_eq!(
        forward.dirty.get("composed"),
        Some(&vec![delta]),
        "a zero-margin composed edge must project the exact delta, not a widened one"
    );
}

/// Route 3 (widened by `r` + margins): the composed node's forward-projected
/// dirt from a replayed upstream delta strictly CONTAINS the exact delta —
/// the plan's own wording, "route 3 asserts the r-widened containment, not
/// equality."
#[test]
fn composed_projection_widened_route_is_a_strict_superset_of_the_exact_delta() {
    let edges = vec![edge("source", "composed", 3, 1)];
    let delta = iv(10, 12);
    let forward = propagate(&edges, &deltas(&[("source", delta)])).expect("propagate");
    let dirty = forward
        .dirty
        .get("composed")
        .expect("composed must be dirty");
    assert_eq!(dirty, &vec![iv(9, 15)]);
    assert!(
        dirty[0].start <= delta.start && delta.end <= dirty[0].end && dirty[0] != delta,
        "route 3's widened projection must strictly contain (not equal) the exact delta: \
         {dirty:?} vs {delta:?}"
    );
}

/// A bare keyed node (no admitted time axis) still refuses fail-loud, in
/// both directions, with a message naming the missing time axis and the
/// composed-shape fix rather than the old bare "keyed-grain" wording.
#[test]
fn bare_keyed_node_still_refuses_with_refined_message() {
    let mut e = edge("source", "bare_keyed", 0, 0);
    e.downstream_grain = PartitionGrain::Keyed;
    let edges = vec![e];

    let fwd = propagate(&edges, &deltas(&[("source", iv(1, 2))]))
        .expect_err("bare keyed node must refuse");
    assert!(fwd.contains("without an admitted time axis"), "{fwd}");
    assert!(fwd.contains("timeseries"), "{fwd}");
    assert!(fwd.contains("bare_keyed"), "{fwd}");

    let bwd =
        required_inputs(&edges, "bare_keyed", iv(1, 2)).expect_err("bare keyed node must refuse");
    assert!(bwd.contains("without an admitted time axis"), "{bwd}");
}

// ---------------------------------------------------------------------------
// Phase D3 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`):
// the adjointness law extended over an observed-delta-fed composed edge —
// the origin's own delta is no longer a hand-typed `DayInterval`, but
// `project_observed_delta`'s own output (a composed model's recorded
// key-level observed delta, projected to partition-day intervals via its
// established locality route). `forward(backward(P)) ⊇ P` must still hold
// when replayed through that projected, possibly-multi-interval delta.
// ---------------------------------------------------------------------------

/// Route 1/2 (`LocalitySlice::Window`/`DeltaValues`, exact projection): a
/// composed origin's recorded observed delta (3 distinct touched
/// partitions) projects to exactly those 3 one-day intervals; replaying
/// them forward through the composed edge must still cover each of the 3
/// periods `required_inputs` resolves for `rollup`.
#[test]
fn observed_delta_fed_composed_edge_satisfies_adjointness_exact_route() {
    let edges = vec![
        edge("source", "composed", 0, 0),
        edge("composed", "rollup", 0, 0),
    ];
    let slice = LocalitySlice::Window {
        partition_column: "d".to_string(),
        margin_before: smelt_logical::Seconds::ZERO,
        margin_after: smelt_logical::Seconds::ZERO,
        recurrence_bounded: false,
    };
    let partitions = vec![
        "2026-01-10".to_string(),
        "2026-01-12".to_string(),
        "2026-01-15".to_string(),
    ];
    let projected = project_observed_delta(&slice, &partitions);
    assert_eq!(
        projected.len(),
        3,
        "3 distinct partitions project to 3 distinct intervals"
    );

    // Every one of the 3 projected intervals must independently satisfy
    // `forward(backward(P)) ⊇ P` at `rollup` when replayed through the
    // SAME `source -> composed -> rollup` chain — the projected delta is
    // fed at `composed`'s own axis (the observed delta is recorded at the
    // composed model's own output, per `docs/specs/incremental_models.md`
    // §"The graph layer" — "Observed deltas on model edges").
    for period in &projected {
        let resolved = required_inputs(&edges, "rollup", *period).expect("resolve");
        let composed_required = resolved
            .required
            .get("composed")
            .cloned()
            .unwrap_or_default();
        let replay: BTreeMap<String, Vec<DayInterval>> =
            [("composed".to_string(), composed_required)]
                .into_iter()
                .collect();
        let forward = propagate(&edges, &replay).expect("propagate");
        let dirty = forward.dirty.get("rollup").unwrap_or_else(|| {
            panic!("rollup must be dirty after replaying its own resolved composed requirement")
        });
        assert!(
            dirty
                .iter()
                .any(|d| d.start <= period.start && period.end <= d.end),
            "forward(backward({period:?})) must cover {period:?} at rollup; got {dirty:?}"
        );
    }
}

/// Route 3 (`LocalitySlice::RecurrenceBounded`, widened projection): the
/// observed delta's single touched partition widens backward by `r` +
/// margins before it ever reaches the graph — the widened interval (not
/// the raw observed day) is what must satisfy adjointness through the
/// composed edge, proving the law holds over the widened form too, not
/// just the exact one the previous test pins.
#[test]
fn observed_delta_fed_composed_edge_satisfies_adjointness_widened_route() {
    let edges = vec![
        edge("source", "composed", 0, 0),
        edge("composed", "rollup", 0, 0),
    ];
    let slice = LocalitySlice::RecurrenceBounded {
        partition_column: "d".to_string(),
        margin_before: smelt_logical::Seconds::days(5),
        margin_after: smelt_logical::Seconds::ZERO,
        r: smelt_logical::Seconds::days(4),
    };
    let partitions = vec!["2026-01-10".to_string()];
    let projected = project_observed_delta(&slice, &partitions);
    assert_eq!(projected.len(), 1);
    let period = projected[0];
    // Widened, not exact: the projected interval spans more than the
    // observed single day.
    assert!(
        period.end - period.start > 1,
        "route 3 must widen: {period:?}"
    );

    let resolved = required_inputs(&edges, "rollup", period).expect("resolve");
    let composed_required = resolved
        .required
        .get("composed")
        .cloned()
        .unwrap_or_default();
    let replay: BTreeMap<String, Vec<DayInterval>> = [("composed".to_string(), composed_required)]
        .into_iter()
        .collect();
    let forward = propagate(&edges, &replay).expect("propagate");
    let dirty = forward
        .dirty
        .get("rollup")
        .expect("rollup must be dirty after replaying the widened requirement");
    assert!(
        dirty
            .iter()
            .any(|d| d.start <= period.start && period.end <= d.end),
        "forward(backward({period:?})) must cover the widened period {period:?} at rollup; \
         got {dirty:?}"
    );
}

// ---------------------------------------------------------------------------
// Phase 5 (`docs/outcomes/20260809-output-delta-typing/outcome.md`): keyed
// dirt-set propagation for admitted shapes — an edge touching a
// `PartitionGrain::Keyed` endpoint whose component vector carries an
// admitted `Addressing::Keyed` component propagates through the keyed
// channel instead of being refused; the refusal narrows to a `General`
// (or absent) verdict.
// ---------------------------------------------------------------------------

fn keyed_component(keys: &[&str]) -> EdgeComponent {
    EdgeComponent {
        group: "{id}".to_string(),
        shape: OutputDelta::KeyedUpsert {
            keys: keys.iter().map(|k| k.to_string()).collect(),
        },
        addressing: Addressing::Keyed {
            keys: keys.iter().map(|k| k.to_string()).collect(),
        },
        columns: keys.iter().map(|k| k.to_string()).collect(),
    }
}

fn general_component(reason: &str) -> EdgeComponent {
    EdgeComponent {
        group: "{weight}".to_string(),
        shape: OutputDelta::General {
            reason: reason.to_string(),
        },
        addressing: Addressing::WholeModel {
            degraded_by: reason.to_string(),
        },
        columns: vec!["weight".to_string()],
    }
}

#[test]
fn keyed_upstream_with_keyed_component_is_not_refused() {
    let mut e = edge("agg", "downstream", 0, 0);
    e.upstream_grain = PartitionGrain::Keyed;
    e.components = vec![keyed_component(&["user_id"])];
    let edges = vec![e];

    propagate(&edges, &deltas(&[("agg", iv(1, 2))]))
        .expect("admitted keyed edge must propagate, not refuse");
}

#[test]
fn keyed_node_with_general_component_still_refuses_naming_the_operator() {
    let mut e = edge("agg", "downstream", 0, 0);
    e.upstream_grain = PartitionGrain::Keyed;
    e.components = vec![general_component("'agg' reads a mutable snapshot")];
    let edges = vec![e];

    let err = propagate(&edges, &deltas(&[("agg", iv(1, 2))]))
        .expect_err("a General-degraded shape must still refuse");
    assert!(
        err.contains("'agg' reads a mutable snapshot"),
        "refusal must name the degrading operator: {err}"
    );
}

#[test]
fn keyed_node_without_components_fails_closed() {
    let mut e = edge("source", "bare_keyed", 0, 0);
    e.downstream_grain = PartitionGrain::Keyed;
    let edges = vec![e];

    let fwd = propagate(&edges, &deltas(&[("source", iv(1, 2))]))
        .expect_err("bare keyed node with no derived shape must still refuse");
    assert!(fwd.contains("without an admitted time axis"), "{fwd}");
    assert!(fwd.contains("timeseries"), "{fwd}");
    assert!(fwd.contains("bare_keyed"), "{fwd}");

    let bwd = required_inputs(&edges, "bare_keyed", iv(1, 2))
        .expect_err("bare keyed node with no derived shape must still refuse");
    assert!(bwd.contains("without an admitted time axis"), "{bwd}");
}

#[test]
fn keyed_edge_propagates_a_keyed_dirt_set_not_intervals() {
    let mut e = edge("agg", "consumer", 0, 0);
    e.upstream_grain = PartitionGrain::Keyed;
    e.downstream_grain = PartitionGrain::Keyed;
    e.components = vec![keyed_component(&["user_id"])];
    let edges = vec![e];

    let result = propagate(&edges, &deltas(&[("agg", iv(1, 2))])).expect("propagate");
    let keys = result
        .per_edge_keys
        .get(&("consumer".to_string(), "agg".to_string()))
        .expect("keyed channel entry must exist");
    assert_eq!(
        keys,
        &vec![KeyedDirt {
            keys: vec!["user_id".to_string()],
            from: "agg".to_string(),
        }]
    );
    assert!(!result
        .keyed_dirty
        .get("consumer")
        .expect("consumer must be keyed-dirty")
        .is_empty());
    assert!(
        !result
            .per_edge
            .contains_key(&("consumer".to_string(), "agg".to_string())),
        "no interval dirt must be reflected through an edge into a keyed-grain consumer: \
         {result:?}"
    );
}

#[test]
fn keyed_dirt_into_a_clocked_consumer_widens_to_whole() {
    let mut e = edge("agg", "rollup", 0, 0);
    e.upstream_grain = PartitionGrain::Keyed;
    // downstream_grain defaults to Day (a clocked consumer).
    e.components = vec![keyed_component(&["user_id"])];
    let edges = vec![e];

    let result = propagate(&edges, &deltas(&[("agg", iv(1, 2))])).expect("propagate");
    let dirty = result.dirty.get("rollup").expect("rollup must be dirty");
    assert!(
        dirty.iter().any(|d| d.is_whole()),
        "a clocked consumer of a keyed origin must get whole-table dirt: {dirty:?}"
    );
    assert!(!result
        .keyed_dirty
        .get("rollup")
        .expect("the keyed channel must also carry the record")
        .is_empty());
}

#[test]
fn required_inputs_over_a_keyed_ancestor_requires_the_whole_table() {
    let mut e = edge("agg", "rollup", 0, 0);
    e.upstream_grain = PartitionGrain::Keyed;
    e.components = vec![keyed_component(&["user_id"])];
    let edges = vec![e];

    let resolved = required_inputs(&edges, "rollup", iv(10, 12)).expect("resolve");
    let agg_required = resolved.required.get("agg").expect("agg must be required");
    assert!(
        agg_required.iter().any(|d| d.is_whole()),
        "a keyed ancestor must require the whole table: {agg_required:?}"
    );
    assert_eq!(
        resolved.required.get("rollup"),
        Some(&vec![iv(10, 12)]),
        "the target's own interval math must be unchanged"
    );
    assert_eq!(
        resolved.build_order,
        vec!["rollup".to_string()],
        "agg is a keyed origin with no inbound edge — like a raw source, it is \
         staged, not built"
    );
}

/// The adjointness law over a graph mixing a window-addressed edge and a
/// keyed-addressed edge: `bronze -> silver` (ordinary interval math) feeds
/// `silver`'s own dirt, while `agg` (a separate keyed-grain origin, fed
/// directly as a delta — mirroring the real graph shape, where a bare
/// keyed node's own inbound edges are never assembled) feeds `rollup`
/// alongside `silver`.
#[test]
fn adjoint_property_holds_with_keyed_edges_present() {
    let window_edge = edge("bronze", "silver", 0, 2);
    let mut keyed_edge = edge("agg", "rollup", 0, 0);
    keyed_edge.upstream_grain = PartitionGrain::Keyed;
    keyed_edge.components = vec![keyed_component(&["user_id"])];
    let mut silver_edge = edge("silver", "rollup", 1, 0);
    silver_edge.upstream_grain = PartitionGrain::Day;

    let edges = vec![window_edge, keyed_edge, silver_edge];
    let period = iv(20, 21);
    let resolved = required_inputs(&edges, "rollup", period).expect("resolve");

    let mut replay: BTreeMap<String, Vec<DayInterval>> = BTreeMap::new();
    for node in ["bronze", "agg"] {
        if let Some(required) = resolved.required.get(node) {
            replay.insert(node.to_string(), required.clone());
        }
    }

    let forward = propagate(&edges, &replay).expect("propagate");
    let dirty = forward.dirty.get("rollup").cloned().unwrap_or_default();
    assert!(
        dirty
            .iter()
            .any(|d| d.start <= period.start && period.end <= d.end),
        "forward(backward(P)) must contain P: {dirty:?} vs {period:?}"
    );
}
