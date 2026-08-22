//! Spark twin of `maintenance_conformance/gate.rs`'s composed keyed pool.
//! Thin wrapper over `smelt_maintenance_testkit::families::gate_composed`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 4).

use smelt_maintenance_testkit::families::{gate_composed, ConformanceBackend};

use crate::backend::SparkConformanceBackend;

/// Default deterministic case count for
/// `composed_keyed_pool_upholds_equivalence_on_spark` (route 3 only) —
/// `SMELT_CONFORMANCE_SPARK_COMPOSED_CASES` env override.
const DEFAULT_COMPOSED_CASES: usize = 4;

fn composed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_SPARK_COMPOSED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_COMPOSED_CASES)
}

/// `composed_keyed_pool_upholds_equivalence_on_spark` — route 3
/// (direct-driver) only; see `families::gate_composed`'s doc comment for why
/// routes 1/2 are excluded.
#[test]
fn composed_keyed_pool_upholds_equivalence_on_spark() {
    let b = SparkConformanceBackend;
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping composed_keyed_pool_upholds_equivalence_on_spark");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate_composed::run_composed_keyed_pool_upholds_equivalence(
        &b,
        composed_case_count(),
    ))
    .expect("composed route-3 equivalence check failed on Spark");
}

/// `composed_keyed_admission_rate_stays_above_floor_on_spark`. Samples ALL
/// THREE routes — admission is pure classification, never execution.
#[test]
fn composed_keyed_admission_rate_stays_above_floor_on_spark() {
    let b = SparkConformanceBackend;
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping composed_keyed_admission_rate_stays_above_floor_on_spark");
        return;
    }
    let n: usize = std::env::var("SMELT_CONFORMANCE_SPARK_COMPOSED_ADMISSION_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate_composed::run_composed_keyed_admission_rate_stays_above_floor(&b, n))
        .expect("composed-pool admission-rate check failed on Spark");
}
