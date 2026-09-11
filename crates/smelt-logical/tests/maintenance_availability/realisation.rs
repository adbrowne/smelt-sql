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

use smelt_core::config::WarehouseTables;
use smelt_dialect::{BackendCapabilities, SqlDialect};
use smelt_logical::maintenance::availability::{
    realisable_state_structures, resolve_availability, StateAvailability, StateStructure,
};
use smelt_logical::maintenance::{Corner, Technique};

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
/// [`every_claimed_structure_has_a_builder`] red from the other side.
///
/// DuckDB emits all five (`smelt_state::ddl_duckdb`'s `generate_ledger_*`,
/// `generate_observed_delta_*`, `generate_fingerprint_sidecar_*`,
/// `generate_tombstone_*`). BigQuery emits both ledgers and the
/// observed-delta record — what it still lacks is the sidecar and the
/// tombstone ledger:
/// `smelt_state::ddl_bigquery`'s `generate_ledger_*` and
/// `generate_observed_delta_*` carry its GoogleSQL spelling, dispatched by
/// `smelt_state::ledger` and `smelt_state::observed_delta`, and
/// `smelt-backend-bigquery` overrides `execute_write_with_bookkeeping` to run
/// the record and the write in one transaction. Its **reconciliation** ledger
/// took more than the emitters: the never-fold-twice refusal is a `PRIMARY
/// KEY` violation on DuckDB and BigQuery's key is `NOT ENFORCED`, so there the
/// record is `generate_ledger_conditional_insert_sql` (a `MERGE … WHEN NOT
/// MATCHED`) and the refusal is `fold_ledger_delta`'s zero-row abort inside a
/// GoogleSQL multi-statement transaction
/// (`smelt_backend_bigquery::sql::fold_ledger_delta_script`).
/// `ddl_spark.rs` carries schema-evolution DDL only, and Spark has no sound
/// realisation to add: Delta gives per-table atomicity only and no cross-table
/// transaction, so a ledger write and its data write cannot be made atomic
/// (`docs/specs/state.md` §"Which dialects realise which structure").
fn has_emitters(dialect: SqlDialect, structure: StateStructure) -> bool {
    match dialect {
        SqlDialect::DuckDB => true,
        SqlDialect::BigQuery => matches!(
            structure,
            StateStructure::MergeLedger
                | StateStructure::ReconciliationLedger
                | StateStructure::ObservedOutputDeltas
        ),
        SqlDialect::SparkSQL => false,
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

/// Today's concrete expectation, stated positively so the reopening's
/// remaining phases flip it deliberately rather than by accident: DuckDB
/// realises everything, BigQuery realises both ledgers and the observed-delta
/// record, and Spark realises **nothing** — permanently, not pending.
#[test]
fn each_dialect_realises_exactly_the_structures_it_has_today() {
    assert!(
        realised(SqlDialect::SparkSQL).is_empty(),
        "Spark's absence is permanent (no cross-table Delta transaction), not pending; got {:?}",
        realised(SqlDialect::SparkSQL),
    );
    assert_eq!(
        realised(SqlDialect::BigQuery),
        BTreeSet::from([
            StateStructure::MergeLedger,
            StateStructure::ReconciliationLedger,
            StateStructure::ObservedOutputDeltas
        ]),
        "BigQuery realises both ledgers and the observed-delta record; its two remaining \
         rows (fingerprint sidecar, tombstone ledger) land with \
         docs/outcomes/20260906-bigquery-correctness phase 15 and later work",
    );
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

/// The consequence of BigQuery's rows, at the layer users feel it: neither a
/// technique needing the merge ledger nor one needing the reconciliation
/// ledger is coarsened any more, and the `KeyedFold` cell that used to be
/// downgraded to `PerGroupRecompute` now survives as itself.
///
/// The **absence** of a downgrade is the assertion — the same shape phase 13
/// established for the `ColumnScopedMerge` half. Spark's half below is what
/// keeps it non-vacuous.
#[test]
fn bigquery_keeps_both_a_merge_ledger_and_a_keyed_fold_technique() {
    let available = StateAvailability::resolve(
        WarehouseTables::Allowed,
        &realisable_state_structures(SqlDialect::BigQuery),
    );

    let mut merge_cell = vec![super::base_cell(
        Corner::ColumnMerge,
        Technique::ColumnScopedMerge,
    )];
    resolve_availability(&mut merge_cell, &available);
    assert_eq!(merge_cell[0].technique, Technique::ColumnScopedMerge);
    assert!(
        merge_cell[0].state_downgrade.is_none(),
        "BigQuery realises the merge ledger, so nothing is lost: {:?}",
        merge_cell[0].state_downgrade,
    );

    let mut fold_cell = vec![super::base_cell(Corner::FoldDelta, Technique::KeyedFold)];
    resolve_availability(&mut fold_cell, &available);
    assert_eq!(
        fold_cell[0].technique,
        Technique::KeyedFold,
        "BigQuery refuses a repeat fold by zero-row abort inside a transaction, so the cell \
         must not be coarsened to a recompute",
    );
    assert!(
        fold_cell[0].state_downgrade.is_none(),
        "no reconciliation-ledger downgrade may be recorded on BigQuery any more: {:?}",
        fold_cell[0].state_downgrade,
    );
}

/// Non-vacuity for the test above: a dialect that genuinely cannot refuse a
/// repeat still downgrades the same cell, and still says which structure it is
/// missing. Spark's absence is permanent — Delta has no cross-table
/// transaction — so this half is not waiting on a future phase.
#[test]
fn a_dialect_without_the_reconciliation_ledger_still_downgrades_a_keyed_fold() {
    let available = StateAvailability::resolve(
        WarehouseTables::Allowed,
        &realisable_state_structures(SqlDialect::SparkSQL),
    );

    let mut fold_cell = vec![super::base_cell(Corner::FoldDelta, Technique::KeyedFold)];
    resolve_availability(&mut fold_cell, &available);
    assert_eq!(fold_cell[0].technique, Technique::PerGroupRecompute);
    assert_eq!(
        fold_cell[0].state_downgrade.as_ref().unwrap().missing,
        StateStructure::ReconciliationLedger,
    );
}
