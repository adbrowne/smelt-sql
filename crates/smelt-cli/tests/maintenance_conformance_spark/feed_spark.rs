//! Spark twin of `maintenance_conformance/gate.rs`'s `change_feed`-source
//! admission leg. Thin wrapper over
//! `smelt_maintenance_testkit::families::feed`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 4).
//!
//! `feed_declared_source_upholds_equivalence_via_recompute` is NOT ported
//! here — see `smelt_maintenance_testkit::families::feed`'s doc comment.

use smelt_maintenance_testkit::families::{feed, ConformanceBackend};

use crate::backend::SparkConformanceBackend;

/// Default deterministic case count for
/// `change_feed_source_admits_recompute_only_on_spark` —
/// `SMELT_CONFORMANCE_SPARK_FEED_ADMISSION_CASES` env override, smaller than
/// the DuckDB leg's 10.
const DEFAULT_CASES: usize = 6;

fn case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_SPARK_FEED_ADMISSION_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

/// `change_feed_source_admits_recompute_only_on_spark`.
#[test]
fn change_feed_source_admits_recompute_only_on_spark() {
    let b = SparkConformanceBackend;
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping change_feed_source_admits_recompute_only_on_spark");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(feed::run_change_feed_source_admits_recompute_only(
        &b,
        case_count(),
    ))
    .expect("change_feed admission check failed on Spark");
}
