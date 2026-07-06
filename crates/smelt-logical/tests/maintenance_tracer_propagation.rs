//! Tracer bullet, series 3: cross-model dirty-partition propagation.
//!
//! Runs start from *what changed upstream*: given the partition intervals
//! that landed per source, compute which partitions of every downstream
//! model must run, by composing each edge's derived scan clamp through the
//! graph (scan → footprint reflection per edge, merged dirt per model,
//! topological order). Day-granular v0; grain mapping (daily → monthly) is
//! the named next step.

use std::collections::BTreeMap;

use smelt_logical::maintenance::derive::{derive_maintenance_plan, ModelInputs};
use smelt_logical::maintenance::propagate::{
    normalize, propagate, required_inputs, DayInterval, Edge,
};
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MutationProfile, OutputSpec, SourceFacts, Trigger,
};
use smelt_logical::Seconds;

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
    }
}

// ---------------------------------------------------------------------------
// The edge clamp comes from the derivation, not a hand-typed number: derive
// the conversions → silver cell (the evolution model's 14d forward window)
// and build the edge from its ScanClamp.
// ---------------------------------------------------------------------------

#[test]
fn derived_conversions_clamp_drives_the_propagation() {
    let sql = "SELECT e.event_id, CAST(e.event_ts AS DATE) AS event_date, \
               (SELECT c.score FROM smelt.sources.conversions c \
                 WHERE c.event_id = e.event_id \
                   AND c.conversion_ts >= e.event_ts \
                   AND c.conversion_ts < e.event_ts + INTERVAL '14 days' \
                   AND c.conversion_date BETWEEN CAST(e.event_ts AS DATE) \
                                             AND CAST(e.event_ts AS DATE) + INTERVAL '14 days' \
                 ORDER BY c.conversion_ts LIMIT 1) AS conversion_score \
               FROM smelt.sources.bronze_events e";
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "silver_events".to_string(),
            grain: Grain::Partition {
                partition_col: "event_date".to_string(),
            },
            skeleton_columns: ["event_id", "event_date"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        },
        sources: vec![SourceFacts {
            name: "conversions".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("conversion_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        }],
        column_groups: vec![ColumnGroup {
            columns: vec!["conversion_score".to_string()],
            mutation_sensitivity: ["conversions"].iter().map(|s| s.to_string()).collect(),
        }],
        fold: None,
        column_add_proof: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::UpstreamMutation {
            source: "conversions".to_string(),
        }],
    );
    let clamp = plan.cells[0]
        .scans
        .iter()
        .find(|c| c.source == "conversions")
        .expect("conversions clamp");
    assert_eq!(clamp.after, Seconds::days(14));

    let e = Edge::from_clamp("silver_events", clamp);
    assert_eq!((e.before_days, e.after_days), (0, 14));

    // One new conversions day D (= day 20) dirties silver [D − 14, D + 1).
    let result = propagate(&[e], &deltas(&[("conversions", iv(20, 21))])).expect("propagate");
    assert_eq!(result.dirty["silver_events"], vec![iv(6, 21)]);
    assert_eq!(
        result.per_edge[&("silver_events".to_string(), "conversions".to_string())],
        vec![iv(6, 21)]
    );
}

// ---------------------------------------------------------------------------
// Chained reflection: conversions → silver (0, 14d) → rollup (0, 0) →
// report (7d lookback). Each hop composes the previous model's dirt.
// ---------------------------------------------------------------------------

#[test]
fn dirt_composes_through_a_chain() {
    let edges = vec![
        edge("conversions", "silver", 0, 14),
        edge("silver", "rollup", 0, 0),
        edge("rollup", "report", 7, 0),
    ];
    let result = propagate(&edges, &deltas(&[("conversions", iv(20, 21))])).expect("propagate");
    assert_eq!(result.dirty["silver"], vec![iv(6, 21)]);
    assert_eq!(
        result.dirty["rollup"],
        vec![iv(6, 21)],
        "same-axis hop is identity"
    );
    assert_eq!(
        result.dirty["report"],
        vec![iv(6, 28)],
        "a 7d lookback extends the dirt forward: rollup days up to 20 feed report days up to 27"
    );
}

// ---------------------------------------------------------------------------
// Multiple sources land in the same tick: per-edge dirt stays separate (it
// keys the trigger cell), while the model's merged dirt is the union.
// ---------------------------------------------------------------------------

#[test]
fn deltas_from_two_sources_merge_per_model_but_stay_separate_per_edge() {
    let edges = vec![
        edge("bronze_events", "silver", 0, 2), // 48h lateness clamp
        edge("conversions", "silver", 0, 14),
    ];
    let result = propagate(
        &edges,
        &deltas(&[("bronze_events", iv(20, 21)), ("conversions", iv(20, 21))]),
    )
    .expect("propagate");
    assert_eq!(
        result.per_edge[&("silver".to_string(), "bronze_events".to_string())],
        vec![iv(18, 21)],
        "an arrival day dirties the event days it can carry (48h back)"
    );
    assert_eq!(
        result.per_edge[&("silver".to_string(), "conversions".to_string())],
        vec![iv(6, 21)]
    );
    assert_eq!(result.dirty["silver"], vec![iv(6, 21)], "union of the two");
}

#[test]
fn disjoint_deltas_stay_disjoint_and_adjacent_ones_merge() {
    let edges = vec![edge("src", "m", 0, 0)];
    let result = propagate(
        &edges,
        &deltas(&[("src", iv(0, 1)), ("src", iv(10, 11)), ("src", iv(11, 12))]),
    )
    .expect("propagate");
    assert_eq!(result.dirty["m"], vec![iv(0, 1), iv(10, 12)]);
}

// ---------------------------------------------------------------------------
// Fan-out and diamond shapes: a model's dirt reaches every consumer, and a
// diamond merges at the join point instead of double-counting.
// ---------------------------------------------------------------------------

#[test]
fn fan_out_and_diamond_propagate_correctly() {
    let edges = vec![
        edge("src", "a", 0, 0),
        edge("src", "b", 1, 0), // b reads src with a 1-day lookback
        edge("a", "sink", 0, 0),
        edge("b", "sink", 0, 0),
    ];
    let result = propagate(&edges, &deltas(&[("src", iv(5, 6))])).expect("propagate");
    assert_eq!(result.dirty["a"], vec![iv(5, 6)]);
    assert_eq!(result.dirty["b"], vec![iv(5, 7)]);
    assert_eq!(
        result.dirty["sink"],
        vec![iv(5, 7)],
        "diamond merges at the sink"
    );
}

// ---------------------------------------------------------------------------
// Guardrails: partial-day clamps widen to whole partitions; cycles refuse.
// ---------------------------------------------------------------------------

#[test]
fn partial_day_clamps_ceil_outward() {
    // A 36h clamp must dirty 2 whole partitions — widening is safe,
    // narrowing never is.
    let clamp = smelt_logical::maintenance::ScanClamp {
        source: "src".to_string(),
        column: "d".to_string(),
        before: Seconds::hours(36),
        after: Seconds::ZERO,
    };
    let e = Edge::from_clamp("m", &clamp);
    assert_eq!(e.before_days, 2);
}

#[test]
fn cyclic_graph_is_refused() {
    let edges = vec![edge("a", "b", 0, 0), edge("b", "a", 0, 0)];
    let err = propagate(&edges, &deltas(&[("a", iv(0, 1))])).expect_err("cycle must refuse");
    assert!(err.contains("cycle"));
}

#[test]
fn normalize_merges_overlaps_and_drops_empties() {
    let merged = normalize(vec![iv(5, 5), iv(3, 6), iv(1, 4), iv(8, 9)]);
    assert_eq!(merged, vec![iv(1, 6), iv(8, 9)]);
}

// ---------------------------------------------------------------------------
// Backward resolution (10-dependency-propagation.md §4, S6): build a model
// for a specified period *including upstreams* — the date arithmetic runs
// backwards through the same edge clamps the forward runs reflect.
// ---------------------------------------------------------------------------

#[test]
fn backward_resolution_uses_the_scan_clamp_directly() {
    // silver [4, 7) requires conversions [4, 21) (forward-widening source)
    // and sessions [2, 7) (backward-widening source) — both directions of
    // widening appear in one resolution.
    let edges = vec![
        edge("conversions", "silver", 0, 14),
        edge("sessions", "silver", 2, 0),
    ];
    let result = required_inputs(&edges, "silver", iv(4, 7)).expect("resolve");
    assert_eq!(result.required["silver"], vec![iv(4, 7)]);
    assert_eq!(result.required["conversions"], vec![iv(4, 21)]);
    assert_eq!(result.required["sessions"], vec![iv(2, 7)]);
    assert_eq!(result.build_order, vec!["silver".to_string()]);
}

#[test]
fn backward_resolution_composes_through_the_chain_and_orders_the_build() {
    let edges = vec![
        edge("bronze", "silver", 0, 2),
        edge("conversions", "silver", 0, 14),
        edge("sessions", "silver", 2, 0),
        edge("silver", "rollup", 0, 0),
        edge("rollup", "report", 7, 0),
    ];
    let result = required_inputs(&edges, "report", iv(10, 13)).expect("resolve");
    assert_eq!(result.required["report"], vec![iv(10, 13)]);
    assert_eq!(
        result.required["rollup"],
        vec![iv(3, 13)],
        "the 7d trailing window reaches back"
    );
    assert_eq!(result.required["silver"], vec![iv(3, 13)]);
    assert_eq!(result.required["bronze"], vec![iv(3, 15)]);
    assert_eq!(result.required["conversions"], vec![iv(3, 27)]);
    assert_eq!(result.required["sessions"], vec![iv(1, 13)]);
    assert_eq!(
        result.build_order,
        vec![
            "silver".to_string(),
            "rollup".to_string(),
            "report".to_string()
        ],
        "ancestor models in dependency order, target last"
    );
}

#[test]
fn backward_resolution_ignores_non_ancestors() {
    // A sibling consumer of silver is not an ancestor of rollup and must
    // not appear in the resolution.
    let edges = vec![
        edge("bronze", "silver", 0, 2),
        edge("silver", "rollup", 0, 0),
        edge("silver", "unrelated_report", 7, 0),
        edge("other_src", "unrelated_report", 0, 0),
    ];
    let result = required_inputs(&edges, "rollup", iv(4, 7)).expect("resolve");
    assert!(!result.required.contains_key("unrelated_report"));
    assert!(!result.required.contains_key("other_src"));
    assert_eq!(
        result.build_order,
        vec!["silver".to_string(), "rollup".to_string()]
    );
}

#[test]
fn backward_diamond_merges_requirements_at_the_shared_ancestor() {
    // sink reads a (no margin) and b (1d lookback); both read src. src's
    // requirement is the union of the two paths' widenings.
    let edges = vec![
        edge("src", "a", 0, 0),
        edge("src", "b", 1, 0),
        edge("a", "sink", 0, 0),
        edge("b", "sink", 3, 0),
    ];
    let result = required_inputs(&edges, "sink", iv(10, 11)).expect("resolve");
    assert_eq!(result.required["a"], vec![iv(10, 11)]);
    assert_eq!(result.required["b"], vec![iv(7, 11)]);
    // via a: [10, 11); via b: [6, 11) — merged.
    assert_eq!(result.required["src"], vec![iv(6, 11)]);
}

// ---------------------------------------------------------------------------
// Adjointness (10-dependency-propagation.md §2, S8): the two directions are
// adjoint, not inverse — replaying the resolved inputs forward dirties at
// least the requested period.
// ---------------------------------------------------------------------------

#[test]
fn forward_of_backward_covers_the_requested_period() {
    let edges = vec![
        edge("bronze", "silver", 0, 2),
        edge("conversions", "silver", 0, 14),
        edge("silver", "rollup", 0, 0),
    ];
    let period = iv(4, 7);
    let resolved = required_inputs(&edges, "rollup", period).expect("resolve");
    // Replay the resolved *source* slices as forward deltas.
    let mut replay: BTreeMap<String, Vec<DayInterval>> = BTreeMap::new();
    for src in ["bronze", "conversions"] {
        replay.insert(src.to_string(), resolved.required[src].clone());
    }
    let forward = propagate(&edges, &replay).expect("propagate");
    let dirty = &forward.dirty["rollup"];
    assert!(
        dirty
            .iter()
            .any(|d| d.start <= period.start && period.end <= d.end),
        "forward(backward([4,7))) must cover [4,7); got {dirty:?}"
    );
    // And strictly more than the period — adjoint, not inverse.
    assert_ne!(dirty, &vec![period]);
}

// ---------------------------------------------------------------------------
// Cascade-as-delta (S5): an upstream *model's* region recompute is itself a
// delta — forward propagation prices the downstream write side of a
// cascading backfill.
// ---------------------------------------------------------------------------

#[test]
fn a_model_region_recompute_cascades_as_a_delta() {
    let edges = vec![
        edge("bronze", "silver", 0, 2),
        edge("silver", "rollup", 0, 0),
        edge("rollup", "report", 7, 0),
    ];
    // silver [4, 7) is being recomputed (a backfill); what must follow?
    let mut deltas: BTreeMap<String, Vec<DayInterval>> = BTreeMap::new();
    deltas.insert("silver".to_string(), vec![iv(4, 7)]);
    let result = propagate(&edges, &deltas).expect("propagate");
    assert_eq!(result.dirty["rollup"], vec![iv(4, 7)]);
    assert_eq!(
        result.dirty["report"],
        vec![iv(4, 14)],
        "the trailing window extends the cascade forward"
    );
    // Nothing upstream of the recomputed region is scheduled.
    assert!(!result
        .per_edge
        .contains_key(&("silver".to_string(), "bronze".to_string())));
}

// ---------------------------------------------------------------------------
// No-op deltas (S4): a source nothing reads, or an empty delta, propagates
// nothing.
// ---------------------------------------------------------------------------

#[test]
fn deltas_nothing_reads_propagate_nothing() {
    let edges = vec![edge("src", "m", 0, 0)];
    let result = propagate(&edges, &deltas(&[("unrelated", iv(5, 6))])).expect("propagate");
    assert!(!result.dirty.contains_key("m"));
    assert!(result.per_edge.is_empty());

    let empty = propagate(&edges, &deltas(&[("src", iv(5, 5))])).expect("propagate");
    assert!(!empty.dirty.contains_key("m"));
}

#[test]
fn self_edge_is_refused_as_a_cycle() {
    // A self-referential model is a cycle in the *table* graph even though
    // it is a DAG in time — v0 refuses; the unrolling is 10-dependency-
    // propagation.md §6.
    let edges = vec![edge("rolling", "rolling", 1, 0)];
    assert!(propagate(&edges, &deltas(&[("rolling", iv(0, 1))])).is_err());
    assert!(required_inputs(&edges, "rolling", iv(0, 1)).is_err());
}

#[test]
fn empty_period_resolves_to_nothing() {
    let edges = vec![edge("src", "m", 0, 0)];
    let result = required_inputs(&edges, "m", iv(7, 7)).expect("resolve");
    assert!(result.required.is_empty());
    assert!(result.build_order.is_empty());
}
