//! BigQuery twin of `maintenance_conformance/gate.rs`'s composed keyed pool.
//! Thin wrapper over `smelt_maintenance_testkit::families::gate_composed`
//! (`docs/plans/20260817-bigquery-generative-conformance.md` Phase 5).
//!
//! Live run (2026-08-17): 1/2 passed
//! (`composed_keyed_admission_rate_stays_above_floor_on_bigquery` — pure
//! classification, no execution). `composed_keyed_pool_upholds_equivalence_on_bigquery`
//! failed that run: the actual COMPILED MODEL SQL (`smelt-runtime`'s
//! `ephemeral_seed_ctes` path, real product code) emitted a `FROM (VALUES
//! ...) AS t(cols)` table constructor GoogleSQL rejects — see `main.rs`'s
//! doc comment point 2. That path now routes through
//! `smelt_core::build_row_set_table`, the single dialect-aware row-set
//! owner, so the compiled SQL no longer contains the rejected construct;
//! this has not yet been re-confirmed with a live re-run of this wrapper.

use smelt_maintenance_testkit::families::{gate_composed, ConformanceBackend};

use crate::backend::BigQueryConformanceBackend;

/// Default deterministic case count for
/// `composed_keyed_pool_upholds_equivalence_on_bigquery` (route 3 only) —
/// `SMELT_CONFORMANCE_BQ_COMPOSED_CASES` env override.
const DEFAULT_COMPOSED_CASES: usize = 2;

fn composed_case_count() -> usize {
    std::env::var("SMELT_CONFORMANCE_BQ_COMPOSED_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_COMPOSED_CASES)
}

/// `composed_keyed_pool_upholds_equivalence_on_bigquery` — route 3
/// (direct-driver) only; see `families::gate_composed`'s doc comment for why
/// routes 1/2 are excluded.
#[test]
fn composed_keyed_pool_upholds_equivalence_on_bigquery() {
    let b = BigQueryConformanceBackend::new("composed_pool");
    if let Some(reason) = b.skip_reason() {
        eprintln!("{reason} — skipping composed_keyed_pool_upholds_equivalence_on_bigquery");
        return;
    }
    b.preflight_or_panic();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate_composed::run_composed_keyed_pool_upholds_equivalence(
        &b,
        composed_case_count(),
    ))
    .expect("composed route-3 equivalence check failed on BigQuery");
}

/// `composed_keyed_admission_rate_stays_above_floor_on_bigquery`. Samples
/// ALL THREE routes — admission is pure classification, never execution.
#[test]
fn composed_keyed_admission_rate_stays_above_floor_on_bigquery() {
    let b = BigQueryConformanceBackend::new("composed_admission");
    if let Some(reason) = b.skip_reason() {
        eprintln!(
            "{reason} — skipping composed_keyed_admission_rate_stays_above_floor_on_bigquery"
        );
        return;
    }
    b.preflight_or_panic();
    let n: usize = std::env::var("SMELT_CONFORMANCE_BQ_COMPOSED_ADMISSION_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(15);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(gate_composed::run_composed_keyed_admission_rate_stays_above_floor(&b, n))
        .expect("composed-pool admission-rate check failed on BigQuery");
}
