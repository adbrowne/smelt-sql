//! Phase 6c (`docs/outcomes/20260913-trino-incremental/phases/06c-plan.md`)
//! — a repair-admitted `PerGroupRecompute` cell requires the fingerprint
//! sidecar unconditionally, and `resolve_availability`'s replacement clears
//! its now-meaningless `ScanClamp`.

use std::collections::HashMap;

use smelt_logical::analysis::affected_keys::DeltaShape;
use smelt_logical::analysis::faithful_fold::{faithful_fold, FaithfulFold};
use smelt_logical::analysis::footprint::FootprintResult;
use smelt_logical::analysis::input_delta::{
    InputDeltaKind, MutationProfile as AnalysisMutationProfile,
};
use smelt_logical::analysis::join_shape::JoinContext;
use smelt_logical::analysis::source_bounds::{BoundResult, Seconds};
use smelt_logical::maintenance::availability::{
    required_state_structure, resolve_availability, StateAvailability, StateStructure,
};
use smelt_logical::maintenance::derive::LocalityInputs;
use smelt_logical::maintenance::repair::{
    admit_per_group_recompute, derive_repair_cell, discovery_posture, has_repair_family_lowering,
    RepairDiscoveryPosture,
};
use smelt_logical::maintenance::{
    KeyDiscovery, KeyScope, MutationProfile, PlanCell, SourceFacts, Technique, Trigger,
};
use smelt_types::SqlFunction;

use super::{base_cell, strings};

fn orders_source() -> SourceFacts {
    SourceFacts {
        name: "orders".to_string(),
        mutation: MutationProfile::MutableSnapshot,
        partition_col: Some("order_date".to_string()),
        unique_key: vec!["order_id".to_string()],
        allow_full_scan: false,
    }
}

/// Build an admitted repair cell over a `MutableSnapshot` source, mirroring
/// `derive_new_data`'s key-grain branch (`repair.rs`'s own module doc) and
/// `repair_cell.rs`'s own fixture shape.
pub(super) fn admitted_repair_cell() -> PlanCell {
    let sql = "SELECT customer_id, MAX(amount) AS max_amount FROM smelt.sources.orders \
               GROUP BY customer_id";
    let mut bounds = HashMap::new();
    bounds.insert(
        "orders".to_string(),
        BoundResult::Bounded {
            source_partition_col: "order_date".to_string(),
            before: Seconds::hours(1),
            after: Seconds::ZERO,
        },
    );
    let footprints: HashMap<String, FootprintResult> = HashMap::new();
    let links = HashMap::new();
    let loc = LocalityInputs {
        bounds: &bounds,
        footprints: &footprints,
        links: &links,
    };
    let delta = DeltaShape {
        source: "orders".to_string(),
        columns: ["customer_id".to_string(), "amount".to_string()]
            .into_iter()
            .collect(),
        keyed: true,
    };
    let admitted = admit_per_group_recompute(
        sql,
        &["customer_id".to_string()],
        &orders_source(),
        None,
        None,
        &loc,
        &delta,
        &JoinContext::new(),
    )
    .expect("obligations 4/6/7 are satisfiable for this fixture");
    derive_repair_cell(
        &admitted,
        Trigger::UpstreamMutation {
            source: "orders".to_string(),
        },
        "{max_amount}".to_string(),
    )
}

/// Test 1: `required_state_structure` over a repair-admitted cell requires
/// the fingerprint sidecar (returned `None` before this phase).
#[test]
fn repair_admitted_cell_requires_the_fingerprint_sidecar() {
    let cell = admitted_repair_cell();
    assert!(cell.key_scope.is_none());
    assert!(!cell.scans.is_empty());
    assert_eq!(
        required_state_structure(&cell),
        Some(StateStructure::FingerprintSidecar)
    );
}

/// Test 2: repair admission is only ever reachable over a `MutableSnapshot`
/// source — an `AppendOnly` source satisfies `faithful_fold`'s condition (1)
/// (so the ordinary fold route is taken, never repair), `MutableSnapshot`'s
/// discovery posture is unconditionally the sidecar diff, and `ChangeFeed`
/// has no discovery posture at all (refused upstream, never reaching a
/// repair cell).
#[test]
fn repair_admission_is_only_ever_over_a_mutable_snapshot_source() {
    let verdict = faithful_fold(
        SqlFunction::Sum,
        false,
        &AnalysisMutationProfile::AppendOnly,
        InputDeltaKind::WindowForward,
    );
    assert!(
        matches!(verdict, FaithfulFold::Holds),
        "an append-only source's partitioned-input condition must hold, so it never falls \
         through to the repair family"
    );

    assert_eq!(
        discovery_posture(MutationProfile::MutableSnapshot),
        Some(RepairDiscoveryPosture::SidecarDiff)
    );
    assert_eq!(discovery_posture(MutationProfile::ChangeFeed), None);
}

/// Test 4: `resolve_availability` with `StateAvailability::none()`
/// downgrades a repair-admitted cell to `DeleteInsert`, records the
/// downgrade naming the missing sidecar, clears `scans`, and the resulting
/// cell has no repair-family lowering.
#[test]
fn repair_cell_downgrades_to_delete_insert_without_the_sidecar() {
    let mut cells = vec![admitted_repair_cell()];
    resolve_availability(&mut cells, &StateAvailability::none());
    assert_eq!(cells[0].technique, Technique::DeleteInsert);
    let downgrade = cells[0].state_downgrade.as_ref().unwrap();
    assert_eq!(downgrade.original, Technique::PerGroupRecompute);
    assert_eq!(downgrade.missing, StateStructure::FingerprintSidecar);
    assert!(cells[0].scans.is_empty());
    assert!(cells[0].key_scope.is_none());
    assert!(!has_repair_family_lowering(&cells[0]));
}

/// Test 4 (availability side): under `StateAvailability::all()` the same
/// cell keeps `PerGroupRecompute` and its `ScanClamp` — no over-firing.
#[test]
fn repair_cell_is_not_downgraded_when_the_sidecar_is_available() {
    let mut cells = vec![admitted_repair_cell()];
    resolve_availability(&mut cells, &StateAvailability::all());
    assert_eq!(cells[0].technique, Technique::PerGroupRecompute);
    assert!(cells[0].state_downgrade.is_none());
    assert!(!cells[0].scans.is_empty());
    assert!(has_repair_family_lowering(&cells[0]));
}

/// Test 5: the clamp-clearing on downgrade is scoped to an
/// `original == PerGroupRecompute` downgrade only — a `ColumnScopedMerge`
/// cell carrying an `EnrichmentKeyed` `key_scope` (phase 3c's route) keeps
/// its unrelated `scans` untouched when it downgrades to `DeleteInsert`.
#[test]
fn a_column_scoped_merge_downgrade_keeps_its_scans() {
    let mut cell = base_cell(
        smelt_logical::maintenance::Corner::ColumnMerge,
        Technique::ColumnScopedMerge,
    );
    cell.key_scope = Some(KeyScope {
        keys: strings(&["repo_id"]),
        from: "gold.repo_dim".to_string(),
        discovery: KeyDiscovery::EnrichmentKeyed,
    });
    cell.scans = vec![smelt_logical::maintenance::ScanClamp {
        source: "orders".to_string(),
        column: "order_date".to_string(),
        before: smelt_logical::analysis::source_bounds::Seconds::ZERO,
        after: smelt_logical::analysis::source_bounds::Seconds::ZERO,
        write_footprint: None,
    }];
    let mut cells = vec![cell];
    resolve_availability(&mut cells, &StateAvailability::none());
    assert_eq!(cells[0].technique, Technique::DeleteInsert);
    let downgrade = cells[0].state_downgrade.as_ref().unwrap();
    assert_eq!(downgrade.original, Technique::ColumnScopedMerge);
    assert!(
        !cells[0].scans.is_empty(),
        "only a PerGroupRecompute original clears scans on downgrade"
    );
}

/// So `trino_invariants.rs` can build the same fixture without duplicating
/// `admit_per_group_recompute`'s call shape.
pub(super) fn admitted_repair_cell_for_trino() -> PlanCell {
    admitted_repair_cell()
}
