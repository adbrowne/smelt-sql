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
        assert!(required_state_structure(cell).is_none());
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

/// A cell whose *ideal* technique is not yet a recompute-family one but
/// which carries a `key_scope` (e.g. `ColumnScopedMerge` over an
/// enrichment-keyed model edge) downgrades to `PerGroupRecompute`, not
/// `DeleteInsert` — the group-scoped repair route is cheaper and the
/// key-scope metadata makes it available. This is distinct from a cell
/// whose technique is *already* `PerGroupRecompute` (see
/// `key_addressed_cell_downgrades_to_delete_insert_without_the_sidecar`
/// below), which has no cheaper recompute-family route left.
#[test]
fn a_cell_with_key_scope_downgrades_to_per_group_recompute() {
    let mut cell = base_cell(Corner::ColumnMerge, Technique::ColumnScopedMerge);
    cell.key_scope = Some(KeyScope {
        keys: strings(&["user_id"]),
        from: "upstream".to_string(),
        discovery: KeyDiscovery::UpstreamKeyed,
    });
    assert_eq!(recompute_equivalent(&cell), Technique::PerGroupRecompute);
}

/// `required_state_structure` step 2 test 1: a `PerGroupRecompute` cell
/// carrying an `UpstreamKeyed` `key_scope` requires the fingerprint sidecar.
#[test]
fn key_addressed_per_group_cell_requires_the_sidecar() {
    let mut cell = base_cell(Corner::ColumnMerge, Technique::PerGroupRecompute);
    cell.key_scope = Some(KeyScope {
        keys: strings(&["user_id"]),
        from: "upstream".to_string(),
        discovery: KeyDiscovery::UpstreamKeyed,
    });
    assert_eq!(
        required_state_structure(&cell),
        Some(StateStructure::FingerprintSidecar)
    );
}

/// Test 2: the same technique **without** a `key_scope` still requires
/// nothing — guards against widening the rule to every recompute cell.
#[test]
fn clamp_bounded_per_group_cell_requires_no_structure() {
    let cell = base_cell(Corner::ColumnMerge, Technique::PerGroupRecompute);
    assert!(cell.key_scope.is_none());
    assert!(required_state_structure(&cell).is_none());
}

/// Test 3: `resolve_availability` with `StateAvailability::none()` downgrades
/// a key-addressed `PerGroupRecompute` cell all the way to `DeleteInsert`,
/// never a no-op back to `PerGroupRecompute`.
#[test]
fn key_addressed_cell_downgrades_to_delete_insert_without_the_sidecar() {
    let mut cells = vec![base_cell(Corner::ColumnMerge, Technique::PerGroupRecompute)];
    cells[0].key_scope = Some(KeyScope {
        keys: strings(&["user_id"]),
        from: "upstream".to_string(),
        discovery: KeyDiscovery::UpstreamKeyed,
    });
    resolve_availability(&mut cells, &StateAvailability::none());
    assert_eq!(cells[0].technique, Technique::DeleteInsert);
    let downgrade = cells[0].state_downgrade.as_ref().unwrap();
    assert_eq!(downgrade.original, Technique::PerGroupRecompute);
    assert_eq!(downgrade.missing, StateStructure::FingerprintSidecar);
}

/// Test 4: the second sidecar-backed discovery route
/// (`DownstreamGrainOverUpstream`) is covered too. `EnrichmentKeyed` is not
/// — it addresses a `ColumnScopedMerge` cell, already `MergeLedger`-gated.
#[test]
fn downstream_grain_over_upstream_cell_requires_the_sidecar() {
    let mut cell = base_cell(Corner::ColumnMerge, Technique::PerGroupRecompute);
    cell.key_scope = Some(KeyScope {
        keys: strings(&["repo_id"]),
        from: "upstream".to_string(),
        discovery: KeyDiscovery::DownstreamGrainOverUpstream,
    });
    assert_eq!(
        required_state_structure(&cell),
        Some(StateStructure::FingerprintSidecar)
    );
}

/// Discovered live (`docs/outcomes/20260912-databricks-dogfood-spine/
/// outcome.md` phase 7b): a `ColumnScopedMerge` cell carrying an
/// `EnrichmentKeyed` `key_scope` (the value-enrichment join shape,
/// `KeyDiscovery::EnrichmentKeyed`'s own doc comment) downgrades all the way
/// to `DeleteInsert` when its required `MergeLedger` is unavailable — never
/// `PerGroupRecompute`, which the key-addressed driver never dispatches for
/// this discovery route. Without this, `recompute_equivalent`'s generic
/// `key_scope.is_some()` rule would hand the driver a `PerGroupRecompute`
/// cell it cannot execute, which then re-hits the sidecar bail this phase
/// exists to eliminate.
#[test]
fn enrichment_keyed_column_scoped_merge_cell_downgrades_to_delete_insert() {
    let mut cells = vec![base_cell(Corner::ColumnMerge, Technique::ColumnScopedMerge)];
    cells[0].key_scope = Some(KeyScope {
        keys: strings(&["repo_id"]),
        from: "gold.repo_dim".to_string(),
        discovery: KeyDiscovery::EnrichmentKeyed,
    });
    resolve_availability(&mut cells, &StateAvailability::none());
    assert_eq!(cells[0].technique, Technique::DeleteInsert);
    let downgrade = cells[0].state_downgrade.as_ref().unwrap();
    assert_eq!(downgrade.original, Technique::ColumnScopedMerge);
    assert_eq!(downgrade.missing, StateStructure::MergeLedger);
}

/// Test 5: under `StateAvailability::all()` the cell keeps
/// `PerGroupRecompute` and carries no downgrade.
#[test]
fn key_addressed_cell_survives_when_the_sidecar_is_available() {
    let mut cells = vec![base_cell(Corner::ColumnMerge, Technique::PerGroupRecompute)];
    cells[0].key_scope = Some(KeyScope {
        keys: strings(&["user_id"]),
        from: "upstream".to_string(),
        discovery: KeyDiscovery::UpstreamKeyed,
    });
    resolve_availability(&mut cells, &StateAvailability::all());
    assert_eq!(cells[0].technique, Technique::PerGroupRecompute);
    assert!(cells[0].state_downgrade.is_none());
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
