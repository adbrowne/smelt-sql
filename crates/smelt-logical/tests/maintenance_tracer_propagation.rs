//! Tracer bullet, series 3: cross-model dependency propagation
//! (`docs/research/20260705-refresh-as-maintenance-plan/10-dependency-propagation.md`).
//!
//! Forward: runs start from *what changed upstream* — per-source deltas
//! reflect through each edge's derived scan clamp to the downstream
//! partitions that must run. Backward: `required_inputs` resolves what must
//! *exist* for a target period (test/validation builds), the date
//! arithmetic running backwards through the same clamps. Day/Month grains
//! align intervals outward per hop.

use std::collections::BTreeMap;

use smelt_logical::maintenance::derive::{derive_maintenance_plan, ModelInputs};
use smelt_logical::maintenance::propagate::{
    civil_from_ordinal, day_ordinal, normalize, ordinal_to_iso, propagate, required_inputs,
    DayInterval, Edge, PartitionGrain,
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
        upstream_grain: PartitionGrain::Day,
        downstream_grain: PartitionGrain::Day,
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

// ---------------------------------------------------------------------------
// Granularity mapping (10-dependency-propagation.md §7, S10): daily models
// feeding monthly reporting models and back. All intervals stay day-ordinal;
// a coarse axis aligns them outward per hop.
// ---------------------------------------------------------------------------

fn month(y: i64, m: u32) -> DayInterval {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    iv(day_ordinal(y, m, 1), day_ordinal(ny, nm, 1))
}

#[test]
fn calendar_math_roundtrips() {
    for ymd in [
        (1970, 1, 1),
        (2026, 1, 1),
        (2026, 2, 28),
        (2024, 2, 29),
        (1999, 12, 31),
        (1, 1, 1),
        (0, 3, 1),
        (-1, 12, 31),
        (2100, 3, 1),
        (2100, 2, 28),
        (2400, 2, 29),
        (2000, 2, 29),
    ] {
        assert_eq!(civil_from_ordinal(day_ordinal(ymd.0, ymd.1, ymd.2)), ymd);
    }
    assert_eq!(day_ordinal(1970, 1, 1), 0);
    assert_eq!(ordinal_to_iso(day_ordinal(2026, 7, 7)), "2026-07-07");
}

#[test]
fn a_dirty_day_dirties_its_whole_containing_month() {
    let mut e = edge("silver", "monthly_report", 0, 0);
    e.downstream_grain = PartitionGrain::Month;
    let d17 = day_ordinal(2026, 1, 17);
    let result = propagate(&[e], &deltas(&[("silver", iv(d17, d17 + 1))])).expect("propagate");
    assert_eq!(result.dirty["monthly_report"], vec![month(2026, 1)]);
}

#[test]
fn a_dirty_december_day_dirties_only_december_not_a_january_rollover() {
    // A December delta's month-aligned outward bound must land on the
    // following January 1st (year rolls over), never wrapping to month 13
    // of the same year — pins the `ey + 1` (not `ey`) year-rollover arm of
    // `PartitionGrain::Month`'s `align_outward`.
    let mut e = edge("silver", "monthly_report", 0, 0);
    e.downstream_grain = PartitionGrain::Month;
    let dec17 = day_ordinal(2026, 12, 17);
    let result = propagate(&[e], &deltas(&[("silver", iv(dec17, dec17 + 1))])).expect("propagate");
    assert_eq!(result.dirty["monthly_report"], vec![month(2026, 12)]);
    assert_eq!(
        result.dirty["monthly_report"][0],
        iv(day_ordinal(2026, 12, 1), day_ordinal(2027, 1, 1)),
        "December's outward-aligned month must end at 2027-01-01, not wrap within 2026"
    );
}

#[test]
fn a_trailing_window_crossing_the_month_boundary_dirties_both_months() {
    // The monthly report reads the daily rollup with a 7d trailing window:
    // a rollup day near the end of January feeds early-February report rows,
    // so both months are dirty.
    let mut e = edge("rollup", "monthly_report", 7, 0);
    e.downstream_grain = PartitionGrain::Month;
    let jan28 = day_ordinal(2026, 1, 28);
    let result = propagate(&[e], &deltas(&[("rollup", iv(jan28, jan28 + 1))])).expect("propagate");
    assert_eq!(
        result.dirty["monthly_report"],
        vec![iv(day_ordinal(2026, 1, 1), day_ordinal(2026, 3, 1))]
    );
}

#[test]
fn a_changed_month_of_a_coarse_dim_dirties_all_its_days_downstream() {
    let mut e = edge("monthly_dim", "daily_model", 0, 0);
    e.upstream_grain = PartitionGrain::Month;
    // A mid-month delta on the dim is a whole-month delta by definition —
    // the seed aligns outward to the source's grain.
    let jan20 = day_ordinal(2026, 1, 20);
    let result =
        propagate(&[e], &deltas(&[("monthly_dim", iv(jan20, jan20 + 1))])).expect("propagate");
    assert_eq!(result.dirty["daily_model"], vec![month(2026, 1)]);
}

#[test]
fn backward_request_for_a_month_reaches_the_days_that_feed_it() {
    let mut e_report = edge("rollup", "monthly_report", 7, 0);
    e_report.downstream_grain = PartitionGrain::Month;
    let edges = vec![edge("silver", "rollup", 0, 0), e_report];
    let result = required_inputs(&edges, "monthly_report", month(2026, 2)).expect("resolve");
    let expected = iv(day_ordinal(2026, 1, 25), day_ordinal(2026, 3, 1));
    assert_eq!(
        result.required["rollup"],
        vec![expected],
        "February's report needs rollup days back to Jan 25 (7d trailing window)"
    );
    assert_eq!(result.required["silver"], vec![expected]);
    // A misaligned request (part of February) is a request for the whole
    // month — the seed aligns outward to the target's grain.
    let partial = required_inputs(
        &edges,
        "monthly_report",
        iv(day_ordinal(2026, 2, 3), day_ordinal(2026, 2, 10)),
    )
    .expect("resolve");
    assert_eq!(partial.required["monthly_report"], vec![month(2026, 2)]);
    assert_eq!(partial.required["rollup"], vec![expected]);
}

#[test]
fn backward_through_a_monthly_dim_requires_whole_months() {
    let mut e = edge("monthly_dim", "daily_model", 0, 0);
    e.upstream_grain = PartitionGrain::Month;
    let result = required_inputs(
        &[e],
        "daily_model",
        iv(day_ordinal(2026, 1, 20), day_ordinal(2026, 2, 10)),
    )
    .expect("resolve");
    assert_eq!(
        result.required["monthly_dim"],
        vec![iv(day_ordinal(2026, 1, 1), day_ordinal(2026, 3, 1))],
        "a day-range request touching two months requires both whole months of the dim"
    );
}

// ---------------------------------------------------------------------------
// S13 (ratified P6): a delta on an unclocked source dirties the WHOLE model
// for every mutation-sensitive consumer — never a silent no-op — and flows
// downstream as whole-table dirt; backward, the required unclocked slice is
// the whole table.
// ---------------------------------------------------------------------------

fn unclocked_edge(upstream: &str, downstream: &str) -> Edge {
    Edge {
        upstream: upstream.to_string(),
        downstream: downstream.to_string(),
        before_days: 0,
        after_days: 0,
        upstream_grain: PartitionGrain::Unclocked,
        downstream_grain: PartitionGrain::Day,
    }
}

#[test]
fn s13_unclocked_dim_delta_dirties_the_whole_consumer_and_flows_on() {
    let edges = vec![
        unclocked_edge("customers", "orders_tiered"),
        edge("orders_tiered", "rollup", 0, 0),
    ];
    // Any delta on the dim — the interval is meaningless on an unclocked
    // axis and aligns outward to WHOLE.
    let result = propagate(&edges, &deltas(&[("customers", iv(5, 6))])).expect("propagate");
    assert_eq!(result.dirty["orders_tiered"].len(), 1);
    assert!(
        result.dirty["orders_tiered"][0].is_whole(),
        "dim churn dirties the whole consumer, never a silent no-op"
    );
    assert!(
        result.dirty["rollup"][0].is_whole(),
        "whole-table dirt flows downstream as whole-table dirt"
    );
}

#[test]
fn s13_backward_requires_the_whole_unclocked_table() {
    let edges = vec![unclocked_edge("customers", "orders_tiered")];
    let result = required_inputs(&edges, "orders_tiered", iv(4, 7)).expect("resolve");
    assert!(
        result.required["customers"][0].is_whole(),
        "an unclocked source's required slice is the whole table (EX-07's full read)"
    );
    // The target's own requirement stays the requested period.
    assert_eq!(result.required["orders_tiered"], vec![iv(4, 7)]);
}

// ---------------------------------------------------------------------------
// S12 guard (ratified P7): a keyed-grain node in the graph refuses fail-loud
// in both directions — silently treating it as a day axis would be
// wrong-and-quiet.
// ---------------------------------------------------------------------------

#[test]
fn s12_keyed_node_refuses_in_both_directions() {
    let mut keyed_edge = edge("payments", "lifetime_spend", 0, 0);
    keyed_edge.downstream_grain = PartitionGrain::Keyed;
    let edges = vec![keyed_edge, edge("lifetime_spend", "report", 0, 0)];

    let fwd =
        propagate(&edges, &deltas(&[("payments", iv(5, 6))])).expect_err("keyed node must refuse");
    assert!(fwd.contains("keyed"), "{fwd}");
    assert!(fwd.contains("lifetime_spend"), "{fwd}");

    let bwd = required_inputs(&edges, "report", iv(4, 7)).expect_err("keyed node must refuse");
    assert!(bwd.contains("keyed"), "{bwd}");
}
