//! The plan layer's claim about what a dialect can build must match what the
//! run layer can actually build (`docs/specs/state.md` §"The state-structure
//! inventory"). A structure a dialect *claims* has emitters and a backend seam
//! behind it; a structure it does not claim produces a recorded downgrade
//! rather than a `bail!`.
//!
//! This is the gate that catches the 2026-09-10 defect
//! (`docs/outcomes/20260906-bigquery-correctness` decision log): BigQuery and
//! Spark claimed `ObservedOutputDeltas` and `FingerprintSidecar` while every
//! emitter of both lives in `smelt_state::ddl_duckdb` behind a DuckDB-only
//! guard, so `resolve_availability` recorded no downgrade and the run refused.

use std::collections::BTreeSet;

use smelt_dialect::{BackendCapabilities, SqlDialect};
use smelt_logical::maintenance::availability::{realisable_state_structures, StateStructure};

/// Every [`SqlDialect`], so a new one is a test failure rather than a silent
/// omission. Kept exhaustive by [`every_dialect_is_covered`].
const ALL_DIALECTS: [SqlDialect; 3] = [
    SqlDialect::DuckDB,
    SqlDialect::SparkSQL,
    SqlDialect::BigQuery,
];

const ALL_STRUCTURES: [StateStructure; 5] = [
    StateStructure::MergeLedger,
    StateStructure::ReconciliationLedger,
    StateStructure::ObservedOutputDeltas,
    StateStructure::FingerprintSidecar,
    StateStructure::TombstoneLedger,
];

/// Does `dialect` actually have emitters for `structure`?
///
/// **This is the table a phase adding a dialect's realisation edits**, and
/// editing it without landing the emitters turns
/// [`every_claimed_structure_has_a_builder`] red from the other side. Today
/// every one of the five structures is emitted only by
/// `smelt_state::ddl_duckdb` (`generate_ledger_*`, `generate_observed_delta_*`,
/// `generate_fingerprint_sidecar_*`, `generate_tombstone_*`); `ddl_bigquery.rs`
/// and `ddl_spark.rs` carry schema-evolution DDL only.
fn has_emitters(dialect: SqlDialect, _structure: StateStructure) -> bool {
    match dialect {
        SqlDialect::DuckDB => true,
        // No ledger, sidecar or observed-delta emitter exists outside
        // `ddl_duckdb`. Spark additionally has no sound realisation to add:
        // Delta gives per-table atomicity only and no cross-table transaction,
        // so a ledger write and its data write cannot be made atomic
        // (`docs/specs/state.md` §"The state-structure inventory").
        SqlDialect::SparkSQL | SqlDialect::BigQuery => false,
    }
}

fn realised(dialect: SqlDialect) -> BTreeSet<StateStructure> {
    realisable_state_structures(dialect).into_iter().collect()
}

/// Two-sided: a claimed structure must have a builder, and a structure with a
/// builder must be claimed. Neither direction may be satisfied by a promise.
#[test]
fn every_claimed_structure_has_a_builder() {
    for dialect in ALL_DIALECTS {
        let claimed = realised(dialect);
        for structure in ALL_STRUCTURES {
            let backed = has_emitters(dialect, structure);
            assert_eq!(
                claimed.contains(&structure),
                backed,
                "{dialect:?} claims={} but backed={backed} for {structure:?} — the plan layer \
                 and the run layer disagree. Either land the emitters or drop the claim; a \
                 claimed-but-unbacked structure makes `resolve_availability` record no \
                 downgrade, and the run refuses instead of degrading.",
                claimed.contains(&structure),
            );
        }
    }
}

/// `FingerprintSidecar` has a second, independent source of truth already in
/// the tree — `BackendCapabilities::supports_fingerprint_sidecar`, which every
/// `_smelt_fingerprint_sidecar` consumer in `maintenance_driver/sidecar.rs`
/// gates on. The availability layer must not contradict it.
#[test]
fn the_sidecar_claim_matches_the_backend_capability() {
    for caps in [
        BackendCapabilities::duckdb(),
        BackendCapabilities::spark_delta(),
        BackendCapabilities::spark_parquet(),
        BackendCapabilities::bigquery(),
    ] {
        let claimed = realised(caps.dialect).contains(&StateStructure::FingerprintSidecar);
        assert_eq!(
            claimed, caps.supports_fingerprint_sidecar,
            "{:?}: realisable_state_structures claims the fingerprint sidecar = {claimed}, but \
             BackendCapabilities::supports_fingerprint_sidecar = {} — a caller asking for a \
             sidecar diff on this dialect fails loud in sidecar.rs while the plan layer thinks \
             the structure is available",
            caps.dialect, caps.supports_fingerprint_sidecar,
        );
    }
}

/// Today's concrete expectation, stated positively so the reopening's phases
/// 12-15 flip it deliberately rather than by accident: BigQuery and Spark
/// realise **nothing**, and DuckDB realises everything.
#[test]
fn bigquery_and_spark_realise_no_state_structure_today() {
    for dialect in [SqlDialect::SparkSQL, SqlDialect::BigQuery] {
        assert!(
            realised(dialect).is_empty(),
            "{dialect:?} should realise no state structure until its emitters land \
             (docs/outcomes/20260906-bigquery-correctness phases 12-15); got {:?}",
            realised(dialect),
        );
    }
    assert_eq!(realised(SqlDialect::DuckDB).len(), ALL_STRUCTURES.len());
}

/// Non-vacuity: [`ALL_DIALECTS`] and [`ALL_STRUCTURES`] must stay exhaustive,
/// or the loops above silently stop covering a variant.
#[test]
fn every_dialect_is_covered() {
    // A new `SqlDialect` variant makes this `match` a compile error, which is
    // the point — the array above must then grow too.
    for dialect in ALL_DIALECTS {
        match dialect {
            SqlDialect::DuckDB | SqlDialect::SparkSQL | SqlDialect::BigQuery => {}
        }
    }
    for structure in ALL_STRUCTURES {
        match structure {
            StateStructure::MergeLedger
            | StateStructure::ReconciliationLedger
            | StateStructure::ObservedOutputDeltas
            | StateStructure::FingerprintSidecar
            | StateStructure::TombstoneLedger => {}
        }
    }
    assert_eq!(ALL_DIALECTS.len(), 3);
    assert_eq!(ALL_STRUCTURES.len(), 5);
}
