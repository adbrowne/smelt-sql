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
use smelt_logical::maintenance::propagate::{normalize, propagate, DayInterval, Edge};
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
