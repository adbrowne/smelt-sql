//! BigQuery twin of `maintenance_conformance/gate.rs`'s append-only
//! partition pool. Thin wrapper: every case's staging/drive/assert logic
//! lives in `smelt_maintenance_testkit::families::gate`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 5) — this
//! file only constructs a `BigQueryConformanceBackend`, decides the
//! deterministic case count from its own env var, and calls the matching
//! `families::gate::run_*` entry point.
//!
//! Live run (2026-08-17): 2/6 passed
//! (`admission_rate_stays_above_floor_on_bigquery`,
//! `boundary_rows_within_reach_are_reflected_on_bigquery` — neither reaches
//! the S-restricted oracle). The other 4 fail on the `VALUES`-table-
//! constructor gap in `STracker::materialize_s_as_view` (see `main.rs`'s doc
//! comment point 1); `append_only_partition_pool_upholds_equivalence_on_bigquery`
//! additionally hit a `MEDIAN` function gap on one generated case.

use smelt_maintenance_testkit::families::{gate, ConformanceBackend};

use crate::backend::BigQueryConformanceBackend;

/// Default deterministic case count for
/// `append_only_partition_pool_upholds_equivalence_on_bigquery` —
/// `SMELT_CONFORMANCE_BQ_CASES` env override. Smaller than even Spark's
/// default (4): each case round-trips over a live BigQuery warehouse with a
/// 3s-per-modification pacing floor (`bigquery_session::TABLE_MODIFICATION_PACING`)
/// on top of the network round trip.
const DEFAULT_CASES: usize = 2;

fn case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_BQ_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

/// Default deterministic sample size for
/// `admission_rate_stays_above_floor_on_bigquery` —
/// `SMELT_CONFORMANCE_BQ_ADMISSION_N` env override. Admission is pure
/// classification (no backend round trip), so this can stay closer to the
/// DuckDB leg's 50 than the execution-heavy tests above need to.
const ADMISSION_DEFAULT_N: usize = 20;

fn admission_sample_size() -> usize {
    std::env::var("SMELT_CONFORMANCE_BQ_ADMISSION_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(ADMISSION_DEFAULT_N)
}

/// `append_only_partition_pool_upholds_equivalence_on_bigquery`.
#[test]
fn append_only_partition_pool_upholds_equivalence_on_bigquery() {
    let b = BigQueryConformanceBackend::new("gate_pool");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping append_only_partition_pool_upholds_equivalence_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate::run_append_only_partition_pool(&b, case_count()))
        .expect("append-only partition pool equivalence check failed on BigQuery");
}

/// `admission_rate_stays_above_floor_on_bigquery`.
#[test]
fn admission_rate_stays_above_floor_on_bigquery() {
    let b = BigQueryConformanceBackend::new("gate_admission");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping admission_rate_stays_above_floor_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate::run_admission_rate_stays_above_floor(
        &b,
        admission_sample_size(),
    ))
    .expect("admission rate check failed on BigQuery");
}

/// `redelivery_of_processed_window_is_idempotent_on_bigquery`.
#[test]
fn redelivery_of_processed_window_is_idempotent_on_bigquery() {
    let b = BigQueryConformanceBackend::new("gate_redelivery");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping redelivery_of_processed_window_is_idempotent_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate::run_redelivery_of_processed_window_is_idempotent(&b))
        .expect("redelivery idempotency check failed on BigQuery");
}

/// `full_refresh_interleave_resets_state_correctly_on_bigquery`.
#[test]
fn full_refresh_interleave_resets_state_correctly_on_bigquery() {
    let b = BigQueryConformanceBackend::new("gate_full_refresh");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping full_refresh_interleave_resets_state_correctly_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate::run_full_refresh_interleave_resets_state_correctly(&b))
        .expect("full-refresh interleave check failed on BigQuery");
}

/// `boundary_rows_within_reach_are_reflected_on_bigquery`.
#[test]
fn boundary_rows_within_reach_are_reflected_on_bigquery() {
    let b = BigQueryConformanceBackend::new("gate_boundary");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping boundary_rows_within_reach_are_reflected_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate::run_boundary_rows_within_reach_are_reflected(&b))
        .expect("boundary-row reach check failed on BigQuery");
}

/// `column_add_between_runs_recovers_equivalence_on_bigquery`.
#[test]
fn column_add_between_runs_recovers_equivalence_on_bigquery() {
    let b = BigQueryConformanceBackend::new("gate_column_add");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping column_add_between_runs_recovers_equivalence_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate::run_column_add_between_runs_recovers_equivalence(&b))
        .expect("column-add recovery check failed on BigQuery");
}
