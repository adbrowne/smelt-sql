//! BigQuery twin of `maintenance_conformance/gate.rs`'s fact+mutable-
//! dimension mixed pool. Thin wrapper over
//! `smelt_maintenance_testkit::families::gate_mixed`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 5).
//!
//! The `open_backend`/`case` and `USING DELTA` defects this file originally
//! documented are fixed upstream (`docs/plans/20260817-bigquery-generative-conformance.md`).
//! Live run (2026-08-17): still fails, on the `VALUES`-table-constructor gap
//! in `STracker::materialize_s_as_view` — see `main.rs`'s doc comment point
//! 1. Staging itself (the part this file used to be blocked on) now succeeds.

use smelt_maintenance_testkit::families::{gate_mixed, ConformanceBackend};

use crate::backend::BigQueryConformanceBackend;

/// Default deterministic case count for
/// `mutable_pool_settles_to_full_refresh_on_bigquery` —
/// `SMELT_CONFORMANCE_BQ_MIXED_CASES` env override.
const MIXED_DEFAULT_CASES: usize = 1;

fn mixed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_BQ_MIXED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MIXED_DEFAULT_CASES)
}

/// `mutable_pool_settles_to_full_refresh_on_bigquery`.
#[test]
fn mutable_pool_settles_to_full_refresh_on_bigquery() {
    let b = BigQueryConformanceBackend::new("mixed_pool");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping mutable_pool_settles_to_full_refresh_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate_mixed::run_mutable_pool_settles_to_full_refresh(
        &b,
        mixed_case_count(),
    ))
    .expect(
        "mutable-dimension settle check failed on BigQuery — expected until \
         STracker::materialize_s_as_view's VALUES-table-constructor SQL is made GoogleSQL-valid \
         (see this file's doc comment)",
    );
}
