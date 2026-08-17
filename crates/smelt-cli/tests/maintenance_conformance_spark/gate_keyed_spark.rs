//! Spark twin of `maintenance_conformance/gate.rs`'s `grain: key` pool.
//! Thin wrapper over `smelt_maintenance_testkit::families::gate_keyed`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 4).

use smelt_maintenance_testkit::families::{gate_keyed, ConformanceBackend};

use crate::backend::SparkConformanceBackend;

/// Default deterministic case count for
/// `keyed_pool_upholds_end_state_equivalence_on_spark` —
/// `SMELT_CONFORMANCE_SPARK_KEYED_CASES` env override. Smaller than the
/// DuckDB leg's default (6): each case drives several `execute_project`
/// windows over a live Spark Connect server.
const DEFAULT_KEYED_CASES: usize = 3;

fn keyed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_SPARK_KEYED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_KEYED_CASES)
}

/// `keyed_pool_upholds_end_state_equivalence_on_spark`.
#[test]
fn keyed_pool_upholds_end_state_equivalence_on_spark() {
    let b = SparkConformanceBackend;
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping keyed_pool_upholds_end_state_equivalence_on_spark");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate_keyed::run_keyed_pool_upholds_end_state_equivalence(
        &b,
        keyed_case_count(),
    ))
    .expect("keyed end-state equivalence check failed on Spark");
}
