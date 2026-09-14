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
const ALL_DIALECTS: [SqlDialect; 4] = [
    SqlDialect::DuckDB,
    SqlDialect::SparkSQL,
    SqlDialect::BigQuery,
    SqlDialect::Trino,
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
/// **Phase 5 (`docs/outcomes/20260913-trino-ledger/outcome.md`) trimmed this
/// to the one row with no `smelt-state` builder.** The other four rows
/// (`MergeLedger`, `ReconciliationLedger`, `ObservedOutputDeltas`,
/// `TombstoneLedger`) used to be restated here by hand — exactly the second
/// source of truth that let the 2026-09-10 BigQuery/Spark defect happen (this
/// module's own header). They are now derived by calling the real
/// `smelt-state` builders directly:
/// `crates/smelt-runtime/tests/availability_seam/builders.rs`'s
/// `a_builder_answers_exactly_when_its_structure_is_claimed`, over the same
/// [`CENSUS`](../../../smelt-runtime/tests/availability_seam/builders.rs)
/// this file no longer restates.
///
/// `FingerprintSidecar` has no `smelt-state` builder at all — DuckDB is the
/// only dialect with a fingerprint-sidecar realisation, gated by
/// `BackendCapabilities::supports_fingerprint_sidecar` and checked against
/// [`the_sidecar_claim_matches_the_backend_capability`], widened below to
/// cover Trino too.
fn has_emitters(dialect: SqlDialect, structure: StateStructure) -> bool {
    match structure {
        StateStructure::FingerprintSidecar => dialect == SqlDialect::DuckDB,
        StateStructure::MergeLedger
        | StateStructure::ReconciliationLedger
        | StateStructure::ObservedOutputDeltas
        | StateStructure::TombstoneLedger => {
            realisable_state_structures(dialect).contains(&structure)
        }
    }
}

fn realised(dialect: SqlDialect) -> BTreeSet<StateStructure> {
    realisable_state_structures(dialect).into_iter().collect()
}

/// Two-sided for [`StateStructure::FingerprintSidecar`], the one row
/// [`has_emitters`] still answers independently of
/// `realisable_state_structures` itself: a claimed structure must have a
/// builder, and a structure with a builder must be claimed. The other four
/// rows would compare `has_emitters` against itself now that phase 5 derives
/// them from `realisable_state_structures` directly — that comparison moved
/// to `crates/smelt-runtime/tests/availability_seam/builders.rs`'s
/// `a_builder_answers_exactly_when_its_structure_is_claimed`, which calls the
/// real `smelt-state` builder rather than a restated table.
#[test]
fn every_claimed_structure_has_a_builder() {
    for dialect in ALL_DIALECTS {
        let claimed = realised(dialect);
        let structure = StateStructure::FingerprintSidecar;
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

/// `FingerprintSidecar` has a second, independent source of truth already in
/// the tree — `BackendCapabilities::supports_fingerprint_sidecar`, which every
/// `_smelt_fingerprint_sidecar` consumer in `maintenance_driver/sidecar.rs`
/// gates on. The availability layer must not contradict it.
///
/// Widened to `trino_iceberg()` by phase 5
/// (`docs/outcomes/20260913-trino-ledger/outcome.md`) — before this the loop
/// checked every dialect `maintenance_driver/sidecar.rs` actually gates on
/// EXCEPT Trino, so a `supports_fingerprint_sidecar` regression on Trino
/// specifically would have passed silently.
#[test]
fn the_sidecar_claim_matches_the_backend_capability() {
    for caps in [
        BackendCapabilities::duckdb(),
        BackendCapabilities::spark_delta(),
        BackendCapabilities::spark_parquet(),
        BackendCapabilities::bigquery(),
        BackendCapabilities::trino_iceberg(),
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
/// realises everything, BigQuery realises both ledgers, the observed-delta
/// record and the tombstone ledger, and Spark realises **nothing** —
/// permanently, not pending.
#[test]
fn each_dialect_realises_exactly_the_structures_it_has_today() {
    assert!(
        realised(SqlDialect::SparkSQL).is_empty(),
        "Spark's absence is permanent (no cross-table Delta transaction), not pending; got {:?}",
        realised(SqlDialect::SparkSQL),
    );
    assert!(
        realised(SqlDialect::Trino).is_empty(),
        "Trino/Iceberg's absence is permanent (same per-table-commit atomicity as Delta), not \
         pending; got {:?}",
        realised(SqlDialect::Trino),
    );
    assert_eq!(
        realised(SqlDialect::BigQuery),
        BTreeSet::from([
            StateStructure::MergeLedger,
            StateStructure::ReconciliationLedger,
            StateStructure::ObservedOutputDeltas,
            StateStructure::TombstoneLedger
        ]),
        "BigQuery realises both ledgers, the observed-delta record and the tombstone ledger; \
         its one remaining row (the fingerprint sidecar) is out of scope for \
         docs/outcomes/20260906-bigquery-correctness",
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
            SqlDialect::DuckDB
            | SqlDialect::SparkSQL
            | SqlDialect::BigQuery
            | SqlDialect::Trino => {}
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
    assert_eq!(ALL_DIALECTS.len(), 4);
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
