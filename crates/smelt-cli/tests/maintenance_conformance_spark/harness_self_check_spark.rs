//! Spark twin of `maintenance_conformance/harness_self_check.rs`. Thin
//! wrapper over `smelt_maintenance_testkit::families::harness_self_check`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 4).

use smelt_maintenance_testkit::families::{harness_self_check, ConformanceBackend};

use crate::backend::SparkConformanceBackend;

/// `oracle_flags_a_seeded_divergence_on_spark`.
#[test]
fn oracle_flags_a_seeded_divergence_on_spark() {
    let b = SparkConformanceBackend;
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping oracle_flags_a_seeded_divergence_on_spark");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(harness_self_check::run_oracle_flags_a_seeded_divergence(&b))
        .expect("harness self-check failed on Spark");
}
