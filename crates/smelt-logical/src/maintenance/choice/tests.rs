use super::*;
use crate::maintenance::{Corner, MaintenancePlan, PartitionLocal, PlanCell};
use smelt_core::config::MaintenanceCellConfig;

fn admitted_plan(source: &str, technique: Technique, corner: Corner) -> MaintenancePlan {
    MaintenancePlan {
        cells: vec![PlanCell {
            group: "{tier}".to_string(),
            trigger: Trigger::UpstreamMutation {
                source: source.to_string(),
            },
            corner,
            technique,
            partition_local: PartitionLocal::Yes,
            scans: vec![],
            ledger_catch_up: false,
            row_identity: crate::maintenance::RowIdentityVerdict {
                identity: crate::maintenance::RowIdentity::WholeRow,
                proven_mismatch: None,
            },
            skeleton_source_closure: None,
            fingerprint_projections: std::collections::BTreeMap::new(),
            key_scope: None,
            state_downgrade: None,
        }],
        refusals: vec![],
        key_locality: None,
    }
}

#[test]
fn pin_bypasses_cost_model_but_not_admission() {
    let plan = admitted_plan("users", Technique::ColumnScopedMerge, Corner::ColumnMerge);
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };

    // A pin naming the admitted technique succeeds, bypassing whatever
    // the cost model would otherwise have chosen.
    let overrides = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::RederiveColumns),
    };
    let resolved = resolve_cell_choice(plan.cell_for(&trigger), &trigger, &overrides, None, true)
        .expect("pin naming the admitted technique must resolve");
    assert_eq!(
        resolved,
        ChosenTechnique::Admitted(Technique::ColumnScopedMerge)
    );

    // Pinning a technique the plan did NOT admit for this cell (a keyed
    // fold, when the cell only admits column-scoped merge) is a hard
    // error, never a silent override.
    let bad_overrides = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::Fold),
    };
    let err = resolve_cell_choice(
        plan.cell_for(&trigger),
        &trigger,
        &bad_overrides,
        None,
        true,
    )
    .expect_err("pinning an unadmitted technique must refuse");
    assert!(err.to_string().contains("MaintenanceUnboundedFootprint"));

    // Pinning `rederive_columns` when the backend cannot run it is the
    // same refusal shape — a capability gap is indistinguishable from
    // an unadmitted cell.
    let err2 = resolve_cell_choice(plan.cell_for(&trigger), &trigger, &overrides, None, false)
        .expect_err("pin naming a capability-gapped backend must refuse");
    assert!(err2.to_string().contains("MaintenanceUnboundedFootprint"));

    // `recompute` is always in the resolvable set — pinning it always
    // succeeds, admitted or not.
    let recompute_overrides = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::Recompute),
    };
    let resolved = resolve_cell_choice(
        plan.cell_for(&trigger),
        &trigger,
        &recompute_overrides,
        None,
        true,
    )
    .expect("recompute is always resolvable");
    assert_eq!(resolved, ChosenTechnique::RegionRecompute);
}

#[test]
fn unadmitted_cell_pin_refuses() {
    // No cell at all for this trigger (the plan refused it upstream).
    let plan = MaintenancePlan::default();
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };
    let overrides = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::RederiveColumns),
    };
    let err = resolve_cell_choice(plan.cell_for(&trigger), &trigger, &overrides, None, true)
        .expect_err("a pin naming a cell the plan never admitted must refuse");
    assert!(err.to_string().contains("MaintenanceUnboundedFootprint"));

    // Absent a pin, the safe default resolves with no error.
    let resolved = resolve_cell_choice(
        plan.cell_for(&trigger),
        &trigger,
        &EffectiveOverride::default(),
        None,
        true,
    )
    .expect("no pin + unadmitted cell must fall back safely, not error");
    assert_eq!(resolved, ChosenTechnique::RegionRecompute);
}

#[test]
fn pin_diff_patch_resolves_to_a_diff_write() {
    let diff_patch_pattern =
        crate::maintenance::lookup_write_pattern("diff_patch").expect("diff_patch registered");
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };

    // A `write: diff_patch` pin over a cell whose admitted technique is
    // a recompute-family member (`DeleteInsert`) resolves to the
    // diff-patch choice, carrying that technique as the recompute base.
    let recompute_plan = admitted_plan("users", Technique::DeleteInsert, Corner::ColumnMerge);
    let resolved = resolve_cell_choice(
        recompute_plan.cell_for(&trigger),
        &trigger,
        &EffectiveOverride::default(),
        Some(diff_patch_pattern),
        true,
    )
    .expect("diff_patch pin over a recompute-family cell must resolve");
    match resolved {
        ChosenTechnique::DiffPatch { recompute, .. } => {
            assert_eq!(recompute, Technique::DeleteInsert);
        }
        other => panic!("expected ChosenTechnique::DiffPatch, got {other:?}"),
    }

    // Also admits a `PerGroupRecompute`-family cell.
    let per_group_plan = admitted_plan("users", Technique::PerGroupRecompute, Corner::ColumnMerge);
    let resolved = resolve_cell_choice(
        per_group_plan.cell_for(&trigger),
        &trigger,
        &EffectiveOverride::default(),
        Some(diff_patch_pattern),
        true,
    )
    .expect("diff_patch pin over a per-group-recompute cell must resolve");
    match resolved {
        ChosenTechnique::DiffPatch { recompute, .. } => {
            assert_eq!(recompute, Technique::PerGroupRecompute);
        }
        other => panic!("expected ChosenTechnique::DiffPatch, got {other:?}"),
    }

    // A `write: diff_patch` pin over a cell whose admitted technique is
    // NOT a recompute-family member (`ColumnScopedMerge`) refuses with a
    // `ChoiceRefusal` — never a silent downgrade to `RegionRecompute` or
    // any other technique.
    let non_recompute_plan =
        admitted_plan("users", Technique::ColumnScopedMerge, Corner::ColumnMerge);
    let err = resolve_cell_choice(
        non_recompute_plan.cell_for(&trigger),
        &trigger,
        &EffectiveOverride::default(),
        Some(diff_patch_pattern),
        true,
    )
    .expect_err("diff_patch pin over a non-recompute-family cell must refuse");
    assert!(err.to_string().contains("MaintenanceUnboundedFootprint"));
}

#[test]
fn diff_patch_over_a_per_group_recompute_admits_the_delete_leg() {
    let diff_patch_pattern =
        crate::maintenance::lookup_write_pattern("diff_patch").expect("diff_patch registered");
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };
    let plan = admitted_plan("users", Technique::PerGroupRecompute, Corner::ColumnMerge);
    let resolved = resolve_cell_choice(
        plan.cell_for(&trigger),
        &trigger,
        &EffectiveOverride::default(),
        Some(diff_patch_pattern),
        true,
    )
    .expect("diff_patch pin over a per-group-recompute cell must resolve");
    match resolved {
        ChosenTechnique::DiffPatch { delete_leg, .. } => {
            assert_eq!(delete_leg, diff_patch::DeleteLeg::Complete);
        }
        other => panic!("expected ChosenTechnique::DiffPatch, got {other:?}"),
    }
}

#[test]
fn diff_patch_over_a_region_delete_insert_default_admits_the_delete_leg() {
    // The region `DeleteInsert` default's own write-window clamp IS its
    // slice-completeness argument (`docs/outcomes/
    // 20260815-definition-delta-migrate/phases/12-plan.md`), so —
    // unlike an as-yet-unthreaded recompute technique — this delete leg
    // is admitted `Complete`, not degraded to `Omitted`.
    let diff_patch_pattern =
        crate::maintenance::lookup_write_pattern("diff_patch").expect("diff_patch registered");
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };
    let plan = admitted_plan("users", Technique::DeleteInsert, Corner::ColumnMerge);
    let resolved = resolve_cell_choice(
        plan.cell_for(&trigger),
        &trigger,
        &EffectiveOverride::default(),
        Some(diff_patch_pattern),
        true,
    )
    .expect("diff_patch pin over a delete-insert cell must resolve");
    match resolved {
        ChosenTechnique::DiffPatch { delete_leg, .. } => {
            assert_eq!(delete_leg, diff_patch::DeleteLeg::Complete);
        }
        other => panic!("expected ChosenTechnique::DiffPatch, got {other:?}"),
    }
}

fn cell_cfg(
    on: &str,
    columns: &[&str],
    prefer: Option<TechniquePreference>,
    technique: Option<CellTechnique>,
) -> MaintenanceCellConfig {
    MaintenanceCellConfig {
        columns: columns.iter().map(|s| s.to_string()).collect(),
        on: on.to_string(),
        prefer,
        technique,
        write: None,
    }
}

#[test]
fn ladder_narrower_scope_wins() {
    // `defaults.prefer: fold` is the broad default; a `cells[]` entry
    // scoped to this exact cell prefers `recompute` instead — the
    // narrower scope must win.
    let defaults = MaintenanceDefaults {
        prefer: Some(TechniquePreference::Fold),
    };
    let cells = vec![cell_cfg(
        "sources.users",
        &["tier"],
        Some(TechniquePreference::Recompute),
        None,
    )];

    let effective = effective_override(
        Some(&defaults),
        &cells,
        "sources.users",
        &["tier".to_string()],
    );
    assert_eq!(effective.prefer, Some(TechniquePreference::Recompute));

    // A cell with no matching `cells[]` entry falls back to the broad
    // default.
    let effective_unmatched = effective_override(
        Some(&defaults),
        &cells,
        "sources.other",
        &["other_col".to_string()],
    );
    assert_eq!(effective_unmatched.prefer, Some(TechniquePreference::Fold));

    // A `cells[].technique` hard pin coexists with — and, since it's
    // even narrower, wins the same way over — a `cells[].prefer` soft
    // bias on the same entry.
    let cells_with_pin = vec![cell_cfg(
        "sources.users",
        &["tier"],
        Some(TechniquePreference::Recompute),
        Some(CellTechnique::RederiveColumns),
    )];
    let effective_pin = effective_override(
        Some(&defaults),
        &cells_with_pin,
        "sources.users",
        &["tier".to_string()],
    );
    assert_eq!(
        effective_pin.technique,
        Some(CellTechnique::RederiveColumns)
    );

    // End-to-end: the ladder's resolved override feeds
    // `resolve_cell_choice` and actually changes the outcome versus the
    // broad default alone.
    let plan = admitted_plan(
        "sources.users",
        Technique::ColumnScopedMerge,
        Corner::ColumnMerge,
    );
    let trigger = Trigger::UpstreamMutation {
        source: "sources.users".to_string(),
    };
    let resolved = resolve_cell_choice(plan.cell_for(&trigger), &trigger, &effective, None, true)
        .expect("recompute is always resolvable");
    assert_eq!(resolved, ChosenTechnique::RegionRecompute);
}

/// Regression test (`docs/plans/20260808-membership-sensitivity.md`
/// Phase 3 reviewer fix): a trigger with TWO sibling cells — pin
/// resolution must consult BOTH, matching each sibling's own columns,
/// never only the first (`MaintenancePlan::cell_for`'s first-match
/// pitfall). Mirrors the empirically-found bug: `daily_events_
/// enriched`'s `UpstreamMutation(users)` trigger derives a `{user_name}`
/// cell AND an `{event_id, event_type, user_id}` sibling cell — a pin
/// scoped to the SECOND cell's own columns must be consulted (loud
/// refusal for an inadmissible technique, honored for an admissible
/// one), never silently ignored just because it isn't the first cell in
/// `plan.cells`.
#[test]
fn pin_scoped_to_a_sibling_cell_is_consulted_not_only_the_first() {
    fn cell(group: &str, source: &str) -> PlanCell {
        PlanCell {
            group: group.to_string(),
            trigger: Trigger::UpstreamMutation {
                source: source.to_string(),
            },
            corner: Corner::RecomputeRegion,
            technique: Technique::DeleteInsert,
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
    let plan = MaintenancePlan {
        cells: vec![
            cell("{user_name}", "users"),
            cell("{event_id,event_type,user_id}", "users"),
        ],
        refusals: vec![],
        key_locality: None,
    };
    let trigger = Trigger::UpstreamMutation {
        source: "users".to_string(),
    };

    // Every sibling's own derived column group — the SAME shape
    // `maintenance_driver.rs`'s fixed loop builds before consulting any
    // override.
    let sibling_group_columns: Vec<Vec<String>> = plan
        .cells_for(&trigger)
        .map(|c| match c.group.as_str() {
            "{user_name}" => vec!["user_name".to_string()],
            "{event_id,event_type,user_id}" => vec![
                "event_id".to_string(),
                "event_type".to_string(),
                "user_id".to_string(),
            ],
            other => panic!("unexpected group {other}"),
        })
        .collect();

    // --- Inadmissible pin scoped to the SECOND sibling: must refuse. ---
    let inadmissible_cells_cfg = vec![cell_cfg(
        "users",
        &["event_id"],
        None,
        Some(CellTechnique::Fold),
    )];
    assert!(
        unaddressed_technique_pin(&inadmissible_cells_cfg, "users", &sibling_group_columns)
            .is_none(),
        "the pin's columns DO address the second sibling — must not be flagged dangling"
    );
    let mut refused = false;
    for (c, group_columns) in plan.cells_for(&trigger).zip(sibling_group_columns.iter()) {
        let overrides = effective_override(None, &inadmissible_cells_cfg, "users", group_columns);
        let result = resolve_cell_choice(Some(c), &trigger, &overrides, None, true);
        if c.group == "{event_id,event_type,user_id}" {
            let err = result.expect_err(
                "the pin scoped to this sibling's own columns must be consulted and refuse \
                 — Fold is not in this cell's resolvable set {recompute, DeleteInsert}",
            );
            assert!(err.to_string().contains("MaintenanceUnboundedFootprint"));
            refused = true;
        } else {
            result.expect(
                "the FIRST sibling carries no matching override — it must resolve its own \
                 safe default, never see the second sibling's pin",
            );
        }
    }
    assert!(
        refused,
        "the pin scoped to the second sibling's columns must have been consulted"
    );

    // --- Admissible pin scoped to the SECOND sibling: must be honored. ---
    let admissible_cells_cfg = vec![cell_cfg(
        "users",
        &["event_id"],
        None,
        Some(CellTechnique::Recompute),
    )];
    assert!(
        unaddressed_technique_pin(&admissible_cells_cfg, "users", &sibling_group_columns).is_none()
    );
    let mut honored = false;
    for (c, group_columns) in plan.cells_for(&trigger).zip(sibling_group_columns.iter()) {
        let overrides = effective_override(None, &admissible_cells_cfg, "users", group_columns);
        let chosen = resolve_cell_choice(Some(c), &trigger, &overrides, None, true)
            .expect("recompute is always resolvable");
        if c.group == "{event_id,event_type,user_id}" {
            assert_eq!(chosen, ChosenTechnique::RegionRecompute);
            honored = true;
        }
    }
    assert!(honored, "the admissible pin must have been honored");

    // --- A pin naming columns from NEITHER sibling: dangling, refused. ---
    let dangling_cells_cfg = vec![cell_cfg(
        "users",
        &["totally_unrelated_column"],
        None,
        Some(CellTechnique::Fold),
    )];
    let dangling = unaddressed_technique_pin(&dangling_cells_cfg, "users", &sibling_group_columns);
    assert!(
        dangling.is_some(),
        "a hard technique pin naming columns absent from every sibling group must be \
         flagged as dangling, never silently ignored"
    );

    // --- The same dangling pin as a SOFT `prefer` never refuses. ---
    let dangling_prefer_cfg = vec![cell_cfg(
        "users",
        &["totally_unrelated_column"],
        Some(TechniquePreference::Fold),
        None,
    )];
    assert!(
        unaddressed_technique_pin(&dangling_prefer_cfg, "users", &sibling_group_columns).is_none(),
        "a soft `prefer` naming columns absent from every sibling group is not flagged — \
         `prefer` never refuses"
    );
}
