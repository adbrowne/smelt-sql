//! Spark twin of `maintenance_conformance/gate.rs`'s fact+mutable-dimension
//! mixed pool. Thin wrapper over
//! `smelt_maintenance_testkit::families::gate_mixed`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 4).

use smelt_maintenance_testkit::families::{gate_mixed, ConformanceBackend};

use crate::backend::SparkConformanceBackend;

/// Default deterministic case count for
/// `mutable_pool_settles_to_full_refresh_on_spark` —
/// `SMELT_CONFORMANCE_SPARK_MIXED_CASES` env override, smaller than the
/// DuckDB leg's 6.
const MIXED_DEFAULT_CASES: usize = 2;

fn mixed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_SPARK_MIXED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(MIXED_DEFAULT_CASES)
}

/// `mutable_pool_settles_to_full_refresh_on_spark`.
#[test]
fn mutable_pool_settles_to_full_refresh_on_spark() {
    let b = SparkConformanceBackend;
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping mutable_pool_settles_to_full_refresh_on_spark");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate_mixed::run_mutable_pool_settles_to_full_refresh(
        &b,
        mixed_case_count(),
    ))
    .expect("mutable-dimension settle check failed on Spark");
}
