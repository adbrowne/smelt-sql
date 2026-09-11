use std::collections::BTreeSet;

use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::availability::{
    realisable_state_structures, required_state_structure, resolve_availability, StateAvailability,
    StateStructure,
};
use smelt_logical::maintenance::{Corner, Technique};

use super::base_cell;

#[test]
fn succession_patch_requires_the_tombstone_ledger() {
    assert_eq!(
        required_state_structure(Technique::SuccessionPatch),
        Some(StateStructure::TombstoneLedger)
    );
}

/// DuckDB and BigQuery realise the tombstone ledger; Spark does not, and its
/// absence there is permanent rather than pending — Delta has no cross-table
/// transaction, so the tombstone record and the presented `MERGE` cannot land
/// atomically (`docs/specs/state.md` §"Which dialects realise which
/// structure").
#[test]
fn tombstone_ledger_is_realisable_everywhere_but_spark() {
    for dialect in [SqlDialect::DuckDB, SqlDialect::BigQuery] {
        let realised: BTreeSet<StateStructure> =
            realisable_state_structures(dialect).into_iter().collect();
        assert!(
            realised.contains(&StateStructure::TombstoneLedger),
            "{dialect:?} realises the tombstone ledger"
        );
    }
    let spark: BTreeSet<StateStructure> = realisable_state_structures(SqlDialect::SparkSQL)
        .into_iter()
        .collect();
    assert!(!spark.contains(&StateStructure::TombstoneLedger));
    assert!(StateAvailability::all().contains(StateStructure::TombstoneLedger));
}

#[test]
fn succession_cell_downgrades_to_full_refresh_without_a_ledger() {
    let mut cells = vec![base_cell(Corner::FoldDelta, Technique::SuccessionPatch)];
    resolve_availability(&mut cells, &StateAvailability::none());
    assert_eq!(cells[0].technique, Technique::DeleteInsert);
    let downgrade = cells[0].state_downgrade.as_ref().unwrap();
    assert_eq!(downgrade.original, Technique::SuccessionPatch);
    assert_eq!(downgrade.missing, StateStructure::TombstoneLedger);
}

/// BigQuery is deliberately **not** in this list any more: it realises the
/// tombstone ledger, so its `SuccessionPatch` cells keep their technique. The
/// non-vacuity that used to come from BigQuery now comes from Spark and from
/// `warehouse_tables: none`, and the positive claim is asserted directly
/// below.
#[test]
fn succession_downgrade_fires_for_spark_and_warehouse_tables_none() {
    for available in [
        StateAvailability::resolve(
            smelt_core::config::WarehouseTables::Allowed,
            &realisable_state_structures(SqlDialect::SparkSQL),
        ),
        StateAvailability::resolve(
            smelt_core::config::WarehouseTables::None,
            &realisable_state_structures(SqlDialect::DuckDB),
        ),
    ] {
        let mut cells = vec![base_cell(Corner::FoldDelta, Technique::SuccessionPatch)];
        resolve_availability(&mut cells, &available);
        assert_eq!(cells[0].technique, Technique::DeleteInsert);
        assert_eq!(
            cells[0].state_downgrade.as_ref().unwrap().missing,
            StateStructure::TombstoneLedger
        );
    }
}

/// The row-15 claim, stated as an **absence of a downgrade**: a BigQuery
/// project whose warehouse tables are allowed keeps its `SuccessionPatch`
/// cell rather than coarsening it to a full refresh.
#[test]
fn bigquery_keeps_the_succession_patch_technique() {
    let available = StateAvailability::resolve(
        smelt_core::config::WarehouseTables::Allowed,
        &realisable_state_structures(SqlDialect::BigQuery),
    );
    let mut cells = vec![base_cell(Corner::FoldDelta, Technique::SuccessionPatch)];
    resolve_availability(&mut cells, &available);
    assert_eq!(cells[0].technique, Technique::SuccessionPatch);
    assert!(cells[0].state_downgrade.is_none());
}

#[test]
fn a_ledger_less_dialect_realises_no_ledger() {
    // Spark is the ledger-less dialect. BigQuery holds **both** ledgers: the
    // merge ledger since phase 13, and the reconciliation ledger since phase
    // 14, whose never-fold-twice refusal no longer needs the enforced key
    // BigQuery does not have — it is a zero-row abort inside a multi-statement
    // transaction instead (`docs/specs/state.md` §"Which dialects realise
    // which structure"), and the observed-delta table since phase 12 (a
    // `MERGE` upsert whose key set is `ARRAY_AGG(DISTINCT CAST(… AS STRING)
    // IGNORE NULLS)`). What both dialects still lack is the fingerprint
    // sidecar, which is what this test is now about.
    for dialect in [SqlDialect::SparkSQL, SqlDialect::BigQuery] {
        let realised: BTreeSet<StateStructure> =
            realisable_state_structures(dialect).into_iter().collect();
        if dialect == SqlDialect::SparkSQL {
            assert!(!realised.contains(&StateStructure::MergeLedger));
            assert!(!realised.contains(&StateStructure::ReconciliationLedger));
            // Spark's observed-delta absence is permanent for the same
            // reason: no cross-table transaction, so the record and the
            // write it describes cannot commit together.
            assert!(!realised.contains(&StateStructure::ObservedOutputDeltas));
            // …and the tombstone ledger, which BigQuery gained in phase 15
            // and Spark never will, for the same reason.
            assert!(!realised.contains(&StateStructure::TombstoneLedger));
        }
        // Corrected 2026-09-10: this test used to assert the sidecar and the
        // observed-delta table WERE realised on these dialects. They are not —
        // every emitter of both is DuckDB-only, and claiming them suppressed
        // the downgrade that should have been recorded, turning a graceful
        // degradation into a hard `bail!` at run time
        // (`docs/outcomes/20260906-bigquery-correctness` decision log,
        // 2026-09-10). Two-sided coverage lives in `realisation.rs`.
        assert!(!realised.contains(&StateStructure::FingerprintSidecar));
    }
}
