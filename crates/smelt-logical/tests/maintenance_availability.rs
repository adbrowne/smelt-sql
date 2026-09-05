//! Phase 4 (`docs/outcomes/20260904-state-residency/outcome.md`) — availability
//! resolution, step 2 of `docs/specs/state.md` §"The degradation contract".
//! Pure-function coverage of `smelt_logical::maintenance::availability`; no
//! consumer wires this in yet (phase 5).

use std::collections::BTreeSet;

use smelt_dialect::SqlDialect;
use smelt_logical::maintenance::availability::{
    realisable_state_structures, recompute_equivalent, required_state_structure,
    resolve_availability, StateAvailability, StateStructure,
};
use smelt_logical::maintenance::derive::{derive_maintenance_plan, FoldSpec, ModelInputs};
use smelt_logical::maintenance::{
    ColumnGroup, Corner, Grain, KeyDiscovery, KeyScope, MutationProfile, OutputSpec,
    PartitionLocal, PlanCell, RowIdentity, RowIdentityVerdict, SourceFacts, Technique, Trigger,
};
use smelt_types::SqlFunction;

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// A keyed-fold plan: `payments` is append-only, the combiner (`SUM`) is
/// invertible, so `derive_maintenance_plan` admits a `KeyedFold` cell for the
/// creation trigger (mirrors `maintenance_plan_admission.rs::inputs`).
fn keyed_fold_plan() -> smelt_logical::maintenance::MaintenancePlan {
    let inputs = ModelInputs {
        sql: "SELECT user_id, SUM(amount) AS lifetime_spend \
              FROM smelt.sources.payments GROUP BY user_id",
        output: OutputSpec {
            table: "lifetime_spend".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["user_id"]),
            },
            skeleton_columns: set(&["user_id"]),
        },
        sources: vec![SourceFacts {
            name: "payments".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: Some("pay_date".to_string()),
            unique_key: vec![],
            allow_full_scan: false,
        }],
        column_groups: vec![ColumnGroup {
            columns: strings(&["lifetime_spend"]),
            mutation_sensitivity: set(&["payments"]),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: Some(FoldSpec {
            add_columns: vec![("lifetime_spend".to_string(), SqlFunction::Sum)],
        }),
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };
    let trigger = Trigger::NewData {
        source: "payments".to_string(),
    };
    derive_maintenance_plan(&inputs, &[trigger])
}

fn base_cell(corner: Corner, technique: Technique) -> PlanCell {
    PlanCell {
        group: "{amount}".to_string(),
        trigger: Trigger::NewData {
            source: "payments".to_string(),
        },
        corner,
        technique,
        partition_local: PartitionLocal::Yes,
        scans: vec![],
        ledger_catch_up: false,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::WholeRow,
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: std::collections::BTreeMap::new(),
        key_scope: None,
        state_downgrade: None,
    }
}

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
        ]
        .into_iter()
        .collect(),
    );
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
