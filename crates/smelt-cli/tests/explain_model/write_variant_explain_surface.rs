use std::collections::BTreeSet;

use crate::support::synthetic_profile;
use smelt_cli::build_maintenance_plan_report;
use smelt_cli::explain::RelationContractView;
use smelt_db::queries::maintenance::MaintenancePlanResult;
use smelt_logical::analysis::walk::{ColumnComparability, Comparability};
use smelt_logical::maintenance::{
    Corner, MaintenancePlan, PartitionLocal, PlanCell, RowIdentity, RowIdentityVerdict, Technique,
    Trigger,
};

/// `{tier}`, proven `Comparable` — the P3 half of the write-suppression
/// proof, threaded here so `report_for`'s cells (all `Key`-identity,
/// `ColumnScopedMerge`) reach the SAME "admitted, preference/pin
/// decides" branches these tests exercised before `smelt explain`
/// consulted real comparability instead of a `facts.has_identity`-only
/// proxy. `technique_suppress_pin_on_an_incomparable_column_is_a_hard_
/// refusal` below is the one test in this module that deliberately
/// supplies a DIFFERENT (`Incomparable`) vector instead.
fn comparable_tier() -> Vec<ColumnComparability> {
    vec![ColumnComparability {
        output: "tier".to_string(),
        comparability: Comparability::Comparable,
    }]
}

fn key_identity() -> RowIdentityVerdict {
    RowIdentityVerdict {
        identity: RowIdentity::Key(vec!["user_id".to_string()]),
        proven_mismatch: None,
    }
}

fn base_cell(trigger: Trigger, ledger_catch_up: bool) -> PlanCell {
    PlanCell {
        group: "{tier}".to_string(),
        trigger,
        corner: Corner::ColumnMerge,
        technique: Technique::ColumnScopedMerge,
        partition_local: PartitionLocal::Yes,
        scans: vec![],
        ledger_catch_up,
        row_identity: key_identity(),
        skeleton_source_closure: None,
        fingerprint_projections: Default::default(),
        key_scope: None,
        state_downgrade: None,
    }
}

fn report_for(cell: PlanCell) -> String {
    report_for_with_overrides(cell, &[], None, comparable_tier())
        .expect("build_maintenance_plan_report")
}

fn report_for_with_overrides(
    cell: PlanCell,
    cells_cfg: &[smelt_core::config::MaintenanceCellConfig],
    defaults_cfg: Option<&smelt_core::config::MaintenanceDefaults>,
    comparability: Vec<ColumnComparability>,
) -> anyhow::Result<String> {
    use smelt_logical::maintenance::ColumnGroup;

    let result = MaintenancePlanResult {
        plan: MaintenancePlan {
            cells: vec![cell],
            refusals: vec![],
            key_locality: None,
        },
        // `base_cell`'s group is `{tier}` (`ColumnGroup::name()` derives
        // the display name from `columns`), matching the single column
        // this fixture's pin tests target.
        column_groups: vec![ColumnGroup {
            columns: vec!["tier".to_string()],
            mutation_sensitivity: Default::default(),
            membership_sensitivity: BTreeSet::new(),
        }],
        degenerate: vec![],
        state_columns: vec![],
        execution_postures: None,
        is_snapshot_reconcile: None,
        comparability,
        succession_advisories: vec![],
        succession_recipe: None,
    };
    let profile = synthetic_profile(&result, "write_variant_fixture");
    build_maintenance_plan_report(
        "write_variant_fixture",
        &result,
        &RelationContractView::from_facts(None, None),
        &[],
        cells_cfg,
        defaults_cfg,
        None,
        &[],
        &[],
        smelt_core::config::ProbeCadence::PerRun,
        &[],
        None,
        None,
        &profile,
    )
}

/// A steady-state trigger (`Trigger::UpstreamMutation`, no ledger
/// catch-up) over a proven `Key` row identity prefers the
/// change-suppressed matched arm.
#[test]
fn steady_state_trigger_prefers_suppressed() {
    let cell = base_cell(
        Trigger::UpstreamMutation {
            source: "sources.users".to_string(),
        },
        false,
    );
    let report = report_for(cell);
    assert!(
        report.contains("write variant: suppressed (preference"),
        "expected the steady-state trigger to prefer the suppressed matched arm: {report}"
    );
}

/// A definition-change backfill cell (`ledger_catch_up: true`) is
/// admitted but not preferred — first-build posture — even over the
/// same proven `Key` row identity, and even on an otherwise
/// steady-state trigger kind.
#[test]
fn ledger_catch_up_cell_shows_first_build_posture() {
    let cell = base_cell(
        Trigger::UpstreamMutation {
            source: "sources.users".to_string(),
        },
        true,
    );
    let report = report_for(cell);
    assert!(
        report.contains("write variant: unconditional (first-build posture"),
        "expected a definition-change backfill cell to show the first-build posture, not \
             the steady-state preference: {report}"
    );
}

/// No proven row identity (`WholeRow`) never admits the conditional
/// variant at all — the report must show the default, never the
/// preference or first-build lines.
#[test]
fn whole_row_identity_shows_default_not_admitted() {
    let mut cell = base_cell(
        Trigger::UpstreamMutation {
            source: "sources.users".to_string(),
        },
        false,
    );
    cell.row_identity = RowIdentityVerdict {
        identity: RowIdentity::WholeRow,
        proven_mismatch: None,
    };
    let report = report_for(cell);
    assert!(
        report.contains("write variant: unconditional (not admitted"),
        "expected the no-proven-identity default line, never a preference/first-build \
             claim: {report}"
    );
    assert!(!report.contains("write variant: suppressed"));
}

fn cell_cfg_with_technique(
    on: &str,
    technique: smelt_core::config::CellTechnique,
) -> smelt_core::config::MaintenanceCellConfig {
    smelt_core::config::MaintenanceCellConfig {
        columns: vec!["tier".to_string()],
        on: on.to_string(),
        prefer: None,
        technique: Some(technique),
        write: None,
    }
}

/// A `technique: suppress` pin forces the change-suppressed matched arm
/// on for a first-build/definition-change-backfill cell that would
/// otherwise default to unconditional (`ledger_catch_up_cell_shows_
/// first_build_posture` above, absent a pin).
#[test]
fn technique_suppress_pin_shows_suppressed_even_on_first_build_posture() {
    let cell = base_cell(
        Trigger::UpstreamMutation {
            source: "sources.users".to_string(),
        },
        true,
    );
    let cells_cfg = vec![cell_cfg_with_technique(
        "sources.users",
        smelt_core::config::CellTechnique::Suppress,
    )];
    let report = report_for_with_overrides(cell, &cells_cfg, None, comparable_tier())
        .expect("build_maintenance_plan_report");
    assert!(
        report.contains("write variant: suppressed (pinned via `technique: suppress`"),
        "expected the pin to override the first-build-posture default: {report}"
    );
}

/// A `technique: unconditional` pin forces the plain matched arm on a
/// steady-state cell that would otherwise prefer suppression
/// (`steady_state_trigger_prefers_suppressed` above, absent a pin).
#[test]
fn technique_unconditional_pin_shows_unconditional_even_on_steady_state_preference() {
    let cell = base_cell(
        Trigger::UpstreamMutation {
            source: "sources.users".to_string(),
        },
        false,
    );
    let cells_cfg = vec![cell_cfg_with_technique(
        "sources.users",
        smelt_core::config::CellTechnique::Unconditional,
    )];
    let report = report_for_with_overrides(cell, &cells_cfg, None, comparable_tier())
        .expect("build_maintenance_plan_report");
    assert!(
        report.contains("write variant: unconditional (pinned via `technique: unconditional`"),
        "expected the pin to override the steady-state preference: {report}"
    );
}

/// A `technique: suppress` pin over a cell whose write-suppression proof
/// genuinely refuses (P2: no proven row identity, `WholeRow`) is a hard
/// `ChoiceRefusal` — `smelt explain` must propagate it as a real error,
/// never a silently-wrong "suppressed" or "falls back to unconditional"
/// success line (the self-contradictory/silent-success text this test
/// replaces coverage for).
#[test]
fn technique_suppress_pin_on_whole_row_identity_is_a_hard_refusal() {
    // `RowIdentity::WholeRow` (no proven row identity) always resolves
    // `resolve_write_suppression` to `Unconditional` — the P2 check
    // short-circuits before comparability or the column group are even
    // consulted, so a `technique: suppress` pin over this cell is
    // genuinely, always inadmissible. `smelt explain` must propagate
    // that `ChoiceRefusal` as a real error, never a silently-wrong
    // "suppressed" or "falls back to unconditional" success line (the
    // self-contradictory/silent-success text this test replaces
    // coverage for).
    let mut cell = base_cell(
        Trigger::UpstreamMutation {
            source: "sources.users".to_string(),
        },
        false,
    );
    cell.row_identity = RowIdentityVerdict {
        identity: RowIdentity::WholeRow,
        proven_mismatch: None,
    };
    let cells_cfg = vec![cell_cfg_with_technique(
        "sources.users",
        smelt_core::config::CellTechnique::Suppress,
    )];
    let err = report_for_with_overrides(cell, &cells_cfg, None, comparable_tier()).expect_err(
        "an inadmissible `technique: suppress` pin must refuse, never print a \
             success/fallback line",
    );
    let message = err.to_string();
    assert!(
        message.contains("technique: suppress"),
        "expected the refusal to name the pin that could not be honoured: {message}"
    );
}

/// A `technique: suppress` pin over a cell that DOES carry a proven
/// `Key` row identity (P2 holds) but whose compared column is not
/// proven comparable across runs (P3 fails) is the same hard
/// `ChoiceRefusal` as the `WholeRow` case above — `smelt explain` must
/// propagate it too, not only the P2-decidable case
/// (`incremental_models.md` §"Per-cell write addressing" → "User
/// pins").
#[test]
fn technique_suppress_pin_on_an_incomparable_column_is_a_hard_refusal() {
    let cell = base_cell(
        Trigger::UpstreamMutation {
            source: "sources.users".to_string(),
        },
        false,
    );
    let cells_cfg = vec![cell_cfg_with_technique(
        "sources.users",
        smelt_core::config::CellTechnique::Suppress,
    )];
    let incomparable_tier = vec![ColumnComparability {
        output: "tier".to_string(),
        comparability: Comparability::Incomparable,
    }];
    let err = report_for_with_overrides(cell, &cells_cfg, None, incomparable_tier).expect_err(
        "a `technique: suppress` pin over an incomparable compared column must refuse, \
             never print a success/fallback line",
    );
    let message = err.to_string();
    assert!(
        message.contains("technique: suppress"),
        "expected the refusal to name the pin that could not be honoured: {message}"
    );
    assert!(
        message.contains("tier"),
        "expected the refusal to trace back to the incomparable column: {message}"
    );
}
