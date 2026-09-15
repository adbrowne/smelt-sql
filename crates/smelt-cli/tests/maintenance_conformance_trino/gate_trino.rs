//! Trino twin of `maintenance_conformance/gate.rs`'s append-only partition
//! pool. Thin wrapper: every case's staging/drive/assert logic lives in
//! `smelt_maintenance_testkit::families::gate`
//! (`docs/outcomes/20260913-trino-incremental/phases/08-plan.md`) — this
//! file only constructs a `TrinoConformanceBackend`, decides the
//! deterministic case count from its own env var, and calls the matching
//! `families::gate::run_*` entry point.

use smelt_maintenance_testkit::families::{gate, ConformanceBackend};

use crate::backend::TrinoConformanceBackend;

/// Default deterministic case count for
/// `append_only_partition_pool_upholds_equivalence_on_trino` —
/// `SMELT_CONFORMANCE_TRINO_CASES` env override. Smaller than the DuckDB
/// leg's default (12), matching Spark's own precedent: each case round-trips
/// over a real Trino coordinator rather than an in-process DuckDB file.
const DEFAULT_CASES: usize = 4;

fn case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_TRINO_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CASES)
}

/// Default deterministic sample size for
/// `admission_rate_stays_above_floor_on_trino` —
/// `SMELT_CONFORMANCE_TRINO_ADMISSION_N` env override, smaller than the
/// DuckDB leg's 50 — admission is pure classification (no backend round
/// trip), so this can stay closer to the DuckDB default than the
/// execution-heavy tests above need to.
const ADMISSION_DEFAULT_N: usize = 20;

fn admission_sample_size() -> usize {
    std::env::var("SMELT_CONFORMANCE_TRINO_ADMISSION_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(ADMISSION_DEFAULT_N)
}

/// `append_only_partition_pool_upholds_equivalence_on_trino`.
#[test]
fn append_only_partition_pool_upholds_equivalence_on_trino() {
    let b = TrinoConformanceBackend::new("gate_pool");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping append_only_partition_pool_upholds_equivalence_on_trino");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate::run_append_only_partition_pool(&b, case_count()))
        .expect("append-only partition pool equivalence check failed on Trino");
}

/// `admission_rate_stays_above_floor_on_trino`.
#[test]
fn admission_rate_stays_above_floor_on_trino() {
    let b = TrinoConformanceBackend::new("gate_admission");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping admission_rate_stays_above_floor_on_trino");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate::run_admission_rate_stays_above_floor(
        &b,
        admission_sample_size(),
    ))
    .expect("admission rate check failed on Trino");
}

/// `redelivery_of_processed_window_is_idempotent_on_trino`.
#[test]
fn redelivery_of_processed_window_is_idempotent_on_trino() {
    let b = TrinoConformanceBackend::new("gate_redelivery");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping redelivery_of_processed_window_is_idempotent_on_trino");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate::run_redelivery_of_processed_window_is_idempotent(&b))
        .expect("redelivery idempotency check failed on Trino");
}

/// `full_refresh_interleave_resets_state_correctly_on_trino`.
#[test]
fn full_refresh_interleave_resets_state_correctly_on_trino() {
    let b = TrinoConformanceBackend::new("gate_full_refresh");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping full_refresh_interleave_resets_state_correctly_on_trino");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate::run_full_refresh_interleave_resets_state_correctly(&b))
        .expect("full-refresh interleave check failed on Trino");
}

/// `boundary_rows_within_reach_are_reflected_on_trino`.
#[test]
fn boundary_rows_within_reach_are_reflected_on_trino() {
    let b = TrinoConformanceBackend::new("gate_boundary");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping boundary_rows_within_reach_are_reflected_on_trino");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate::run_boundary_rows_within_reach_are_reflected(&b))
        .expect("boundary-row reach check failed on Trino");
}

/// `column_add_between_runs_recovers_equivalence_on_trino`.
#[test]
fn column_add_between_runs_recovers_equivalence_on_trino() {
    let b = TrinoConformanceBackend::new("gate_column_add");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping column_add_between_runs_recovers_equivalence_on_trino");
        return;
    }
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate::run_column_add_between_runs_recovers_equivalence(&b))
        .expect("column-add recovery check failed on Trino");
}
