//! `state_availability` (`docs/specs/state.md` §"The degradation
//! contract"): the two-step ideal-then-availability resolution pass over an
//! already-derived [`MaintenancePlan`].

use smelt_logical::maintenance::availability::{
    resolve_state_availability, StateAvailability, StateStructure,
};
use smelt_logical::maintenance::{
    Corner, MaintenancePlan, PartitionLocal, PlanCell, RecomputeFallback, Refusal, RowIdentity,
    RowIdentityVerdict, ScanClamp, Technique, Trigger,
};
use std::collections::BTreeMap;

fn base_cell(technique: Technique) -> PlanCell {
    PlanCell {
        group: "{total}".to_string(),
        trigger: Trigger::NewData {
            source: "events".to_string(),
        },
        corner: Corner::FoldDelta,
        technique,
        partition_local: PartitionLocal::Yes,
        scans: vec![],
        ledger_catch_up: false,
        row_identity: RowIdentityVerdict {
            identity: RowIdentity::Key(vec!["id".to_string()]),
            proven_mismatch: None,
        },
        skeleton_source_closure: None,
        fingerprint_projections: BTreeMap::new(),
        key_scope: None,
        recompute_fallback: None,
    }
}

fn fallback_scan() -> ScanClamp {
    ScanClamp {
        source: "events".to_string(),
        column: "event_time".to_string(),
        before: smelt_logical::analysis::source_bounds::Seconds::ZERO,
        after: smelt_logical::analysis::source_bounds::Seconds::ZERO,
    }
}

#[test]
fn keyed_fold_without_ledger_downgrades_to_per_group_recompute() {
    let mut cell = base_cell(Technique::KeyedFold);
    cell.recompute_fallback = Some(RecomputeFallback {
        technique: Technique::PerGroupRecompute,
        scans: vec![fallback_scan()],
        key_scope: None,
    });
    let ideal = MaintenancePlan {
        cells: vec![cell],
        refusals: vec![],
        key_locality: None,
    };

    let resolved = resolve_state_availability(&ideal, &StateAvailability::none());

    assert_eq!(resolved.plan.cells.len(), 1);
    assert_eq!(
        resolved.plan.cells[0].technique,
        Technique::PerGroupRecompute
    );
    assert_eq!(resolved.plan.cells[0].scans, vec![fallback_scan()]);
    assert!(resolved.plan.refusals.is_empty());
    assert_eq!(resolved.downgrades.len(), 1);
    let d = &resolved.downgrades[0];
    assert_eq!(d.ideal_technique, Technique::KeyedFold);
    assert_eq!(d.resolved_technique, Technique::PerGroupRecompute);
    assert_eq!(d.missing_structure, StateStructure::ReconciliationLedger);

    // The ideal plan itself must still name the un-downgraded technique.
    assert_eq!(ideal.cells[0].technique, Technique::KeyedFold);
}

#[test]
fn keyed_fold_without_ledger_and_no_fallback_refuses_loudly() {
    let cell = base_cell(Technique::KeyedFold); // recompute_fallback: None
    let ideal = MaintenancePlan {
        cells: vec![cell],
        refusals: vec![],
        key_locality: None,
    };

    let resolved = resolve_state_availability(&ideal, &StateAvailability::none());

    assert!(
        resolved.plan.cells.is_empty(),
        "a keyed fold with no admissible fallback must never run on a ledger-less backend"
    );
    assert_eq!(resolved.plan.refusals.len(), 1);
    match &resolved.plan.refusals[0] {
        Refusal::NoAdmissibleTechnique { why, .. } => {
            assert!(why.contains("reconciliation ledger"), "{why}");
        }
        other => panic!("expected NoAdmissibleTechnique, got {other:?}"),
    }
}

#[test]
fn region_recompute_without_frontier_record_records_a_downgrade() {
    let cell = base_cell(Technique::DeleteInsert);
    let ideal = MaintenancePlan {
        cells: vec![cell],
        refusals: vec![],
        key_locality: None,
    };

    let resolved = resolve_state_availability(
        &ideal,
        &StateAvailability {
            reconciliation_ledger: true,
            frontier_record: false,
            interval_frontier: true,
        },
    );

    // The technique is unaffected — the frontier record is bookkeeping, not
    // a correctness premise.
    assert_eq!(resolved.plan.cells.len(), 1);
    assert_eq!(resolved.plan.cells[0].technique, Technique::DeleteInsert);
    assert!(resolved.plan.refusals.is_empty());
    assert_eq!(resolved.downgrades.len(), 1);
    let d = &resolved.downgrades[0];
    assert_eq!(d.ideal_technique, Technique::DeleteInsert);
    assert_eq!(d.resolved_technique, Technique::DeleteInsert);
    assert_eq!(d.missing_structure, StateStructure::FrontierRecord);
}

#[test]
fn full_availability_is_a_no_op() {
    let mut cell = base_cell(Technique::KeyedFold);
    cell.recompute_fallback = Some(RecomputeFallback {
        technique: Technique::PerGroupRecompute,
        scans: vec![fallback_scan()],
        key_scope: None,
    });
    let ideal = MaintenancePlan {
        cells: vec![cell, base_cell(Technique::DeleteInsert)],
        refusals: vec![],
        key_locality: None,
    };

    let resolved = resolve_state_availability(&ideal, &StateAvailability::all());

    assert_eq!(resolved.plan.cells.len(), ideal.cells.len());
    for (resolved_cell, ideal_cell) in resolved.plan.cells.iter().zip(ideal.cells.iter()) {
        assert_eq!(resolved_cell.technique, ideal_cell.technique);
    }
    assert!(resolved.downgrades.is_empty());
}

#[test]
fn ideal_plan_survives_resolution() {
    let mut cell = base_cell(Technique::KeyedFold); // no fallback -> would drop on resolution
    cell.recompute_fallback = None;
    let ideal = MaintenancePlan {
        cells: vec![cell],
        refusals: vec![],
        key_locality: None,
    };

    let resolved = resolve_state_availability(&ideal, &StateAvailability::none());

    // Resolution dropped the cell (no fallback)...
    assert!(resolved.plan.cells.is_empty());
    // ...but the caller's own `ideal` object is untouched: the
    // counterfactual `smelt explain` print still has it to read.
    assert_eq!(ideal.cells.len(), 1);
    assert_eq!(ideal.cells[0].technique, Technique::KeyedFold);
}
