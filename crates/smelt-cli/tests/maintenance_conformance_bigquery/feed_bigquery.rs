//! BigQuery twin of `maintenance_conformance/gate.rs`'s `change_feed`-source
//! admission leg. Thin wrapper over
//! `smelt_maintenance_testkit::families::feed`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 5).
//!
//! `feed_declared_source_upholds_equivalence_via_recompute` is NOT ported
//! here — see `smelt_maintenance_testkit::families::feed`'s doc comment
//! (the same exclusion the Spark twin makes).
//!
//! Live run (2026-08-17): passed — admission-only (pure classification, no
//! oracle comparison), so it does not reach any of the defects `main.rs`'s
//! doc comment names.

use smelt_maintenance_testkit::families::{feed, ConformanceBackend};

use crate::backend::BigQueryConformanceBackend;

/// Default deterministic case count for
/// `change_feed_source_admits_recompute_only_on_bigquery` —
/// `SMELT_CONFORMANCE_BQ_FEED_ADMISSION_CASES` env override. Admission-only
/// (pure classification), so this can stay closer to the DuckDB leg's 10
/// than the execution-heavy families need to.
const DEFAULT_CASES: usize = 6;

fn case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_BQ_FEED_ADMISSION_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

/// `change_feed_source_admits_recompute_only_on_bigquery`.
#[test]
fn change_feed_source_admits_recompute_only_on_bigquery() {
    let b = BigQueryConformanceBackend::new("feed_admission");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping change_feed_source_admits_recompute_only_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(feed::run_change_feed_source_admits_recompute_only(
        &b,
        case_count(),
    ))
    .expect("change_feed admission check failed on BigQuery");
}
