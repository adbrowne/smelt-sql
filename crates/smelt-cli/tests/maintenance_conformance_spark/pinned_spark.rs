//! Spark twin of `maintenance_conformance/pinned.rs`. Thin wrapper over
//! `smelt_maintenance_testkit::families::pinned`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 4).
//!
//! `hazard::keyed_merge_reprocessed_window` is the ONE hazard case NOT
//! ported — see `smelt_maintenance_testkit::families::pinned`'s doc comment.

use smelt_maintenance_testkit::families::{pinned, ConformanceBackend};

use crate::backend::SparkConformanceBackend;

/// `pinned_recipes_reproduce_catalogue_coverage_on_spark`.
#[test]
fn pinned_recipes_reproduce_catalogue_coverage_on_spark() {
    let b = SparkConformanceBackend;
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping pinned_recipes_reproduce_catalogue_coverage_on_spark");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(pinned::run_pinned_recipes_reproduce_catalogue_coverage(&b))
        .expect("pinned catalogue coverage check failed on Spark");
}

/// `hazard_schedules_are_pinned_on_spark`.
#[test]
fn hazard_schedules_are_pinned_on_spark() {
    let b = SparkConformanceBackend;
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping hazard_schedules_are_pinned_on_spark");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(pinned::run_hazard_schedules_are_pinned(&b))
        .expect("pinned hazard schedule check failed on Spark");
}
