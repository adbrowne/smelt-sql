use std::collections::BTreeSet;

use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::availability::{
    realisable_state_structures, recompute_equivalent, required_state_structure,
    resolve_availability, StateAvailability, StateStructure,
};
use smelt_logical::maintenance::{Corner, KeyDiscovery, KeyScope, Technique};

use super::{base_cell, keyed_fold_plan, strings};

#[test]
fn full_availability_changes_nothing() {
    let mut plan = keyed_fold_plan();
    assert!(!plan.cells.is_empty());
    let before: Vec<_> = plan.cells.iter().map(|c| c.technique).collect();
    resolve_availability(&mut plan.cells, &StateAvailability::all());
    let after: Vec<_> = plan.cells.iter().map(|c| c.technique).collect();
    assert_eq!(before, after);
    assert!(plan.cells.iter().all(|c| c.state_downgrade.is_none()));
}

#[test]
fn keyed_fold_downgrades_to_the_recompute_family() {
    let mut cells = vec![base_cell(Corner::FoldDelta, Technique::KeyedFold)];
    resolve_availability(&mut cells, &StateAvailability::none());
    assert_eq!(cells[0].technique, Technique::PerGroupRecompute);
    let downgrade = cells[0].state_downgrade.as_ref().unwrap();
    assert_eq!(downgrade.original, Technique::KeyedFold);
    assert_eq!(downgrade.missing, StateStructure::ReconciliationLedger);
}

#[test]
fn column_scoped_merge_downgrades_to_the_recompute_family() {
    let mut cells = vec![base_cell(Corner::ColumnMerge, Technique::ColumnScopedMerge)];
    resolve_availability(&mut cells, &StateAvailability::none());
    assert_eq!(cells[0].technique, Technique::PerGroupRecompute);
    let downgrade = cells[0].state_downgrade.as_ref().unwrap();
    assert_eq!(downgrade.original, Technique::ColumnScopedMerge);
    assert_eq!(downgrade.missing, StateStructure::MergeLedger);
}

#[test]
fn region_recompute_cells_require_no_structure() {
    let mut cells = vec![
        base_cell(Corner::RecomputeRegion, Technique::DeleteInsert),
        base_cell(Corner::ColumnMerge, Technique::PerGroupRecompute),
    ];
    resolve_availability(&mut cells, &StateAvailability::none());
    assert_eq!(cells[0].technique, Technique::DeleteInsert);
    assert_eq!(cells[1].technique, Technique::PerGroupRecompute);
    assert!(cells.iter().all(|c| c.state_downgrade.is_none()));
}

#[test]
fn the_record_names_the_original_technique_and_the_missing_structure() {
    let mut cells = vec![base_cell(Corner::FoldDelta, Technique::InPlaceUpdate)];
    resolve_availability(&mut cells, &StateAvailability::none());
    let downgrade = cells[0].state_downgrade.as_ref().unwrap();
    assert_eq!(downgrade.original, Technique::InPlaceUpdate);
    assert_eq!(downgrade.missing, StateStructure::MergeLedger);
    assert!(!downgrade.reason.is_empty());
}

#[test]
fn downgraded_cells_need_no_structure() {
    let mut cells = vec![
        base_cell(Corner::FoldDelta, Technique::KeyedFold),
        base_cell(Corner::ColumnMerge, Technique::ColumnScopedMerge),
    ];
    resolve_availability(&mut cells, &StateAvailability::none());
    for cell in &cells {
        assert!(required_state_structure(cell.technique).is_none());
    }
}

#[test]
fn resolution_is_idempotent() {
    let mut cells = vec![base_cell(Corner::FoldDelta, Technique::KeyedFold)];
    resolve_availability(&mut cells, &StateAvailability::none());
    let after_first = cells[0].clone();
    resolve_availability(&mut cells, &StateAvailability::none());
    assert_eq!(cells[0].technique, after_first.technique);
    assert_eq!(cells[0].state_downgrade, after_first.state_downgrade);
}

#[test]
fn warehouse_tables_none_denies_every_engine_resident_structure() {
    let available = StateAvailability::resolve(
        smelt_core::config::WarehouseTables::None,
        &[
            StateStructure::MergeLedger,
            StateStructure::ReconciliationLedger,
            StateStructure::ObservedOutputDeltas,
            StateStructure::FingerprintSidecar,
        ],
    );
    for structure in [
        StateStructure::MergeLedger,
        StateStructure::ReconciliationLedger,
        StateStructure::ObservedOutputDeltas,
        StateStructure::FingerprintSidecar,
    ] {
        assert!(!available.contains(structure));
    }
}

#[test]
fn ideal_derivation_records_no_downgrade() {
    let plan = keyed_fold_plan();
    assert!(!plan.cells.is_empty());
    assert!(plan.cells.iter().all(|c| c.state_downgrade.is_none()));
}

#[test]
fn a_cell_with_key_scope_downgrades_to_per_group_recompute() {
    let mut cell = base_cell(Corner::ColumnMerge, Technique::PerGroupRecompute);
    cell.key_scope = Some(KeyScope {
        keys: strings(&["user_id"]),
        from: "upstream".to_string(),
        discovery: KeyDiscovery::UpstreamKeyed,
    });
    assert_eq!(recompute_equivalent(&cell), Technique::PerGroupRecompute);
}

#[test]
fn duckdb_realises_every_state_structure() {
    let realised: BTreeSet<StateStructure> = realisable_state_structures(SqlDialect::DuckDB)
        .into_iter()
        .collect();
    assert_eq!(
        realised,
        [
            StateStructure::MergeLedger,
            StateStructure::ReconciliationLedger,
            StateStructure::ObservedOutputDeltas,
            StateStructure::FingerprintSidecar,
            StateStructure::TombstoneLedger,
        ]
        .into_iter()
        .collect(),
    );
}
