//! BigQuery twin of `maintenance_conformance/pinned.rs`. Thin wrapper over
//! `smelt_maintenance_testkit::families::pinned`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 5).
//!
//! `hazard::keyed_merge_reprocessed_window` is the ONE hazard case NOT
//! ported — see `smelt_maintenance_testkit::families::pinned`'s doc comment
//! (the same exclusion the Spark twin makes).
//!
//! The mixed-pool staging defect this file originally documented is fixed
//! upstream. Live run (2026-08-17): both wrappers still fail, but on the
//! FIRST pinned recipe each drives (`recipe_additive_agg` /
//! `recipe_pass_through`, both pass-through/aggregate — neither reaches the
//! mixed-pool path at all) — the `VALUES`-table-constructor gap in
//! `STracker::materialize_s_as_view`, see `main.rs`'s doc comment point 1.

use smelt_maintenance_testkit::families::{pinned, ConformanceBackend};

use crate::backend::BigQueryConformanceBackend;

/// `pinned_recipes_reproduce_catalogue_coverage_on_bigquery`.
#[test]
fn pinned_recipes_reproduce_catalogue_coverage_on_bigquery() {
    let b = BigQueryConformanceBackend::new("pinned_catalogue");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping pinned_recipes_reproduce_catalogue_coverage_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(pinned::run_pinned_recipes_reproduce_catalogue_coverage(&b))
        .expect(
            "pinned catalogue coverage check failed on BigQuery — expected until \
             STracker::materialize_s_as_view's VALUES-table-constructor SQL is made \
             GoogleSQL-valid (see this file's doc comment)",
        );
}

/// `hazard_schedules_are_pinned_on_bigquery`.
#[test]
fn hazard_schedules_are_pinned_on_bigquery() {
    let b = BigQueryConformanceBackend::new("pinned_hazard");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping hazard_schedules_are_pinned_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(pinned::run_hazard_schedules_are_pinned(&b))
        .expect(
            "pinned hazard schedule check failed on BigQuery — expected until \
             STracker::materialize_s_as_view's VALUES-table-constructor SQL is made \
             GoogleSQL-valid (see this file's doc comment)",
        );
}
