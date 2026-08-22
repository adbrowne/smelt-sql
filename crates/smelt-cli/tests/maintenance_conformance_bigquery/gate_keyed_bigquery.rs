//! BigQuery twin of `maintenance_conformance/gate.rs`'s `grain: key` pool.
//! Thin wrapper over `smelt_maintenance_testkit::families::gate_keyed`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 5).
//!
//! Live run (2026-08-17): fails on the `VALUES`-table-constructor gap in
//! `STracker::materialize_s_as_view` — see `main.rs`'s doc comment point 1.

use smelt_maintenance_testkit::families::{gate_keyed, ConformanceBackend};

use crate::backend::BigQueryConformanceBackend;

/// Default deterministic case count for
/// `keyed_pool_upholds_end_state_equivalence_on_bigquery` —
/// `SMELT_CONFORMANCE_BQ_KEYED_CASES` env override. Smaller than Spark's
/// default (3): each case drives several `execute_project` windows over a
/// live BigQuery warehouse with a 3s-per-modification pacing floor.
const DEFAULT_KEYED_CASES: usize = 2;

fn keyed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_BQ_KEYED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_KEYED_CASES)
}

/// `keyed_pool_upholds_end_state_equivalence_on_bigquery`.
#[test]
fn keyed_pool_upholds_end_state_equivalence_on_bigquery() {
    let b = BigQueryConformanceBackend::new("keyed_pool");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping keyed_pool_upholds_end_state_equivalence_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate_keyed::run_keyed_pool_upholds_end_state_equivalence(
        &b,
        keyed_case_count(),
    ))
    .expect("keyed end-state equivalence check failed on BigQuery");
}
