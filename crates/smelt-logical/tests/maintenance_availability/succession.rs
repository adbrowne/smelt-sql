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

#[test]
fn tombstone_ledger_is_realisable_only_on_duckdb() {
    let duckdb: BTreeSet<StateStructure> = realisable_state_structures(SqlDialect::DuckDB)
        .into_iter()
        .collect();
    assert!(duckdb.contains(&StateStructure::TombstoneLedger));
    for dialect in [SqlDialect::SparkSQL, SqlDialect::BigQuery] {
        let realised: BTreeSet<StateStructure> =
            realisable_state_structures(dialect).into_iter().collect();
        assert!(!realised.contains(&StateStructure::TombstoneLedger));
    }
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

#[test]
fn succession_downgrade_fires_for_spark_bigquery_and_warehouse_tables_none() {
    for available in [
        StateAvailability::resolve(
            smelt_core::config::WarehouseTables::Allowed,
            &realisable_state_structures(SqlDialect::SparkSQL),
        ),
        StateAvailability::resolve(
            smelt_core::config::WarehouseTables::Allowed,
            &realisable_state_structures(SqlDialect::BigQuery),
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

#[test]
fn a_ledger_less_dialect_realises_no_ledger() {
    for dialect in [SqlDialect::SparkSQL, SqlDialect::BigQuery] {
        let realised: BTreeSet<StateStructure> =
            realisable_state_structures(dialect).into_iter().collect();
        assert!(!realised.contains(&StateStructure::MergeLedger));
        assert!(!realised.contains(&StateStructure::ReconciliationLedger));
        assert!(realised.contains(&StateStructure::FingerprintSidecar));
        assert!(realised.contains(&StateStructure::ObservedOutputDeltas));
    }
}
