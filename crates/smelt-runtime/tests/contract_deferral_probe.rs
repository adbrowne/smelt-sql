//! Coverage for `smelt_runtime::contract_probes`'s `deferral` probe — the
//! pure builder plus the ledger-frontier evaluation
//! (`docs/specs/incremental_models.md` §"The contract lattice", deferral;
//! `docs/outcomes/20260809-contract-lattice-v1/phases/04-plan.md`). Unlike
//! `frozen_horizon`'s probe, this one reads two already-recorded ledger
//! stores rather than executing SQL, so no DuckDB backend is needed.

use smelt_core::config::{ContractConfig, DataLatency};
use smelt_core::metadata::ModelMetadata;
use smelt_runtime::contract_probes::{deferral_probes, evaluate_deferral};
use smelt_runtime::probes::ProbePolicy;
use smelt_state::intervals::{Interval, IntervalStore, ModelIntervals};
use smelt_state::landed_deltas::{LandedDeltaStore, SourceLanding};
use smelt_state::ProbeRecordOutcome;

fn deferral_metadata(hours: u32) -> ModelMetadata {
    ModelMetadata {
        contract: Some(ContractConfig {
            deferral: DataLatency::parse(&format!("{hours} hours")),
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
fn probe_is_opt_in() {
    let probes = deferral_probes("m", "m", None);
    assert!(
        probes.is_empty(),
        "absent a `contract.deferral` declaration the probe is opt-in only"
    );

    let no_deferral = ModelMetadata::default();
    let probes = deferral_probes("m", "m", Some(&no_deferral));
    assert!(probes.is_empty());
}

#[test]
fn probe_holds_within_the_window() {
    // D = 6 hours -> 1 day (rounded up). Maintained frontier 2026-01-05,
    // input frontier 2026-01-06: lag is 1 day, within D.
    let metadata = deferral_metadata(6);
    let probes = deferral_probes("m", "m", Some(&metadata));
    assert_eq!(probes.len(), 1);

    let interval_store = intervals_ending("m", "2026-01-05");
    let landed_deltas = landed_deltas_ending("raw.events", "2026-01-06");

    let (records, violations) = evaluate_deferral(
        &ProbePolicy::per_run(),
        &probes,
        "m",
        &["raw.events".to_string()],
        &interval_store,
        &landed_deltas,
    );
    assert!(violations.is_empty(), "lag within D must not violate");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].outcome, ProbeRecordOutcome::Dispatched);
}

#[test]
fn probe_raises_when_lag_exceeds_d() {
    // D = 6 hours -> 1 day. Maintained frontier 2026-01-01, input frontier
    // 2026-01-10: lag is 9 days, well past D.
    let metadata = deferral_metadata(6);
    let probes = deferral_probes("m", "m", Some(&metadata));

    let interval_store = intervals_ending("m", "2026-01-01");
    let landed_deltas = landed_deltas_ending("raw.events", "2026-01-10");

    let (_records, violations) = evaluate_deferral(
        &ProbePolicy::per_run(),
        &probes,
        "m",
        &["raw.events".to_string()],
        &interval_store,
        &landed_deltas,
    );
    assert_eq!(violations.len(), 1, "the exceeded lag must be flagged");
    assert!(
        violations[0].message.contains("ContractDeferralExceeded"),
        "{}",
        violations[0].message
    );
    assert!(
        violations[0].message.contains('m'),
        "{}",
        violations[0].message
    );
}

#[test]
fn missing_maintained_frontier_establishes_without_violating() {
    // No recorded intervals for the model at all: first run, nothing to
    // disprove.
    let metadata = deferral_metadata(6);
    let probes = deferral_probes("m", "m", Some(&metadata));

    let interval_store = IntervalStore::default();
    let landed_deltas = landed_deltas_ending("raw.events", "2026-01-10");

    let (_records, violations) = evaluate_deferral(
        &ProbePolicy::per_run(),
        &probes,
        "m",
        &["raw.events".to_string()],
        &interval_store,
        &landed_deltas,
    );
    assert!(violations.is_empty());
}
