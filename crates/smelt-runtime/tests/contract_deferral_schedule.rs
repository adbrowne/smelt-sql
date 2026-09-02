//! Coverage for `smelt_runtime::contract_probes`'s deferral-scheduling
//! builders — the run-skip decision, its propagation to dependents, and
//! the subsumption date-formatting wrapper
//! (`docs/specs/incremental_models.md` §"The contract lattice", deferral;
//! `docs/outcomes/20260809-contract-lattice-v1/phases/05-plan.md`). Pure
//! over already-recorded ledger state — no DuckDB backend needed.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDate;
use smelt_core::config::{ContractCellConfig, ContractConfig, DataLatency};
use smelt_core::metadata::ModelMetadata;
use smelt_logical::contract::deferral::RunLicense;
use smelt_runtime::contract_probes::{
    deferral_cell_decisions, deferral_decision, propagate_deferral_skip, subsumed_window,
};
use smelt_state::intervals::{Interval, IntervalStore, ModelIntervals};
use smelt_state::landed_deltas::{LandedDeltaStore, SourceLanding};

fn deferral_metadata(days: u32) -> ModelMetadata {
    ModelMetadata {
        contract: Some(ContractConfig {
            deferral: DataLatency::parse(&format!("{days} days")),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn intervals_ending(model: &str, end: &str) -> IntervalStore {
    let mut store = IntervalStore::default();
    let mut mi = ModelIntervals::new("hash".to_string());
    mi.covered_intervals.push(Interval {
        start: "2026-01-01".to_string(),
        end: end.to_string(),
    });
    store.models.insert(model.to_string(), mi);
    store
}

fn landed_deltas_ending(source: &str, end: &str) -> LandedDeltaStore {
    let mut store = LandedDeltaStore::default();
    let mut landing = SourceLanding::default();
    landing.covered_intervals.push(Interval {
        start: "2026-01-01".to_string(),
        end: end.to_string(),
    });
    store.sources.insert(source.to_string(), landing);
    store
}

#[test]
fn undeclared_model_is_never_deferral_skipped() {
    let interval_store = intervals_ending("m", "2026-01-01");
    let landed_deltas = landed_deltas_ending("raw.events", "2026-01-10");

    let decision = deferral_decision(
        "m",
        None,
        &["raw.events".to_string()],
        &interval_store,
        &landed_deltas,
    );
    assert!(
        decision.is_none(),
        "a model with no `contract.deferral` declaration must never be scheduled for a skip"
    );

    let no_deferral = ModelMetadata::default();
    let decision = deferral_decision(
        "m",
        Some(&no_deferral),
        &["raw.events".to_string()],
        &interval_store,
        &landed_deltas,
    );
    assert!(decision.is_none());
}

#[test]
fn declaring_model_within_window_is_skipped() {
    // D = 6 days. Maintained frontier 2026-01-05, input frontier 2026-01-08:
    // lag is 3 days, within D.
    let metadata = deferral_metadata(6);
    let interval_store = intervals_ending("m", "2026-01-05");
    let landed_deltas = landed_deltas_ending("raw.events", "2026-01-08");

    let decision = deferral_decision(
        "m",
        Some(&metadata),
        &["raw.events".to_string()],
        &interval_store,
        &landed_deltas,
    )
    .expect("model declares contract.deferral");

    assert_eq!(decision.license, RunLicense::Skip { lag: 3, d: 6 });
    assert!(decision.pending.is_some());
}

#[test]
fn declaring_model_beyond_window_is_not_skipped() {
    // D = 6 days. Maintained frontier 2026-01-01, input frontier 2026-01-20:
    // lag is 19 days, past D — this is the deferral-exceeded violation the
    // probe (not the scheduler) reports; the scheduler must never license a
    // skip here.
    let metadata = deferral_metadata(6);
    let interval_store = intervals_ending("m", "2026-01-01");
    let landed_deltas = landed_deltas_ending("raw.events", "2026-01-20");

    let decision = deferral_decision(
        "m",
        Some(&metadata),
        &["raw.events".to_string()],
        &interval_store,
        &landed_deltas,
    )
    .expect("model declares contract.deferral");

    assert_eq!(decision.license, RunLicense::Run);
}

#[test]
fn deferral_skip_propagates_to_dependents() {
    // upstream -> mid -> downstream. `upstream` is its own deferral skip;
    // `mid` and `downstream` are both transitively downstream of it.
    let mut upstream_map: HashMap<String, HashSet<String>> = HashMap::new();
    upstream_map.insert("upstream".to_string(), HashSet::new());
    upstream_map.insert("mid".to_string(), HashSet::from(["upstream".to_string()]));
    upstream_map.insert(
        "downstream".to_string(),
        HashSet::from(["upstream".to_string(), "mid".to_string()]),
    );
    // `sibling` shares no dependency edge with `upstream` and must not be
    // swept in.
    upstream_map.insert("sibling".to_string(), HashSet::new());

    let own_skip = HashSet::from(["upstream".to_string()]);
    let all_models = vec![
        "upstream".to_string(),
        "mid".to_string(),
        "downstream".to_string(),
        "sibling".to_string(),
    ];

    let skip_set = propagate_deferral_skip(&own_skip, &upstream_map, &all_models);

    assert!(skip_set.contains("upstream"));
    assert!(skip_set.contains("mid"));
    assert!(skip_set.contains("downstream"));
    assert!(
        !skip_set.contains("sibling"),
        "a model outside the deferred model's dependency closure must not be skipped"
    );
}

#[test]
fn covering_run_after_a_recorded_skip_reports_the_subsumed_window() {
    let metadata = deferral_metadata(6);
    let interval_store = intervals_ending("m", "2026-01-05");
    let landed_deltas = landed_deltas_ending("raw.events", "2026-01-08");
    let decision = deferral_decision(
        "m",
        Some(&metadata),
        &["raw.events".to_string()],
        &interval_store,
        &landed_deltas,
    )
    .expect("model declares contract.deferral");

    let start = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let end = NaiveDate::from_ymd_opt(2026, 1, 8).unwrap();

    // No recorded prior skip: never reports subsumption, however wide the
    // covering range is.
    assert!(subsumed_window(decision.pending, false, start, end).is_none());

    // A recorded prior skip plus a covering range: reports the subsumed
    // window.
    let subsumed = subsumed_window(decision.pending, true, start, end)
        .expect("range covers the pending window");
    assert_eq!(subsumed.maintained_exclusive, "2026-01-05");
    assert_eq!(subsumed.input_inclusive, "2026-01-08");

    // A recorded prior skip but a range that does not cover the whole
    // pending window: no subsumption.
    let partial_end = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
    assert!(subsumed_window(decision.pending, true, start, partial_end).is_none());
}

fn cells_metadata(cells: Vec<ContractCellConfig>) -> ModelMetadata {
    ModelMetadata {
        contract: Some(ContractConfig {
            cells,
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn intervals_with_cell_frontier(model: &str, cell_address: &str, end: &str) -> IntervalStore {
    let mut store = IntervalStore::default();
    let mut mi = ModelIntervals::new("hash".to_string());
    mi.record_cell_frontier(cell_address, end);
    store.models.insert(model.to_string(), mi);
    store
}

#[test]
fn per_cell_decision_skips_one_cell_and_runs_its_sibling() {
    use smelt_logical::contract::deferral::cell_address;

    // Two cells, each addressing a different trigger, each with its own D:
    // `on_a` is within its own D (3 <= 6 -> skip); `on_b` is beyond its own
    // D (10 > 2 -> run), even though both cells share the same model.
    let metadata = cells_metadata(vec![
        ContractCellConfig {
            columns: vec!["a".to_string()],
            on: "raw.a".to_string(),
            deferral: DataLatency::parse("6 days"),
        },
        ContractCellConfig {
            columns: vec!["b".to_string()],
            on: "raw.b".to_string(),
            deferral: DataLatency::parse("2 days"),
        },
    ]);

    let addr_a = cell_address(&["a".to_string()], "raw.a");
    let addr_b = cell_address(&["b".to_string()], "raw.b");

    let mut interval_store = intervals_with_cell_frontier("m", &addr_a, "2026-01-05");
    interval_store
        .models
        .get_mut("m")
        .unwrap()
        .record_cell_frontier(&addr_b, "2026-01-01");

    let mut landed_deltas = landed_deltas_ending("raw.a", "2026-01-08");
    landed_deltas.sources.insert(
        "raw.b".to_string(),
        landed_deltas_ending("raw.b", "2026-01-11")
            .sources
            .remove("raw.b")
            .unwrap(),
    );

    let decisions = deferral_cell_decisions("m", Some(&metadata), &interval_store, &landed_deltas);
    assert_eq!(decisions.len(), 2);

    let a = decisions.iter().find(|d| d.address == addr_a).unwrap();
    assert_eq!(a.license, RunLicense::Skip { lag: 3, d: 6 });

    let b = decisions.iter().find(|d| d.address == addr_b).unwrap();
    assert_eq!(b.license, RunLicense::Run);
}

#[test]
fn model_level_deferral_still_decides_when_no_cells_are_declared() {
    // The existing model-level path (`deferral_decision`) is unaffected by
    // `deferral_cell_decisions` — a model with no `contract.cells[]`
    // declares no per-cell decisions at all.
    let metadata = deferral_metadata(6);
    let interval_store = intervals_ending("m", "2026-01-05");
    let landed_deltas = landed_deltas_ending("raw.events", "2026-01-08");

    let decisions = deferral_cell_decisions("m", Some(&metadata), &interval_store, &landed_deltas);
    assert!(decisions.is_empty());

    let decision = deferral_decision(
        "m",
        Some(&metadata),
        &["raw.events".to_string()],
        &interval_store,
        &landed_deltas,
    )
    .expect("model declares contract.deferral");
    assert_eq!(decision.license, RunLicense::Skip { lag: 3, d: 6 });
}
