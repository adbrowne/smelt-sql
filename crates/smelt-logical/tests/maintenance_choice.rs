//! Phase MP13 (`docs/plans/20260707-maintenance-plan-impl.md`) — the
//! technique choice ladder (`incremental_models.md` §Surface "Frontmatter":
//! `defaults.prefer` → `cells[].prefer` → `cells[].technique`, narrower
//! scope winning; a hard pin bypasses the cost model, never admission).
//!
//! These two named tests are the plan's TDD entry points; the module they
//! exercise (`smelt_logical::maintenance::choice`) carries the fuller unit
//! coverage (capability-gap refusal, unadmitted-cell refusal, the ladder's
//! unmatched-cell fallback). This file is the production-facing conformance
//! check: it asserts the exact two named properties the plan phase commits
//! to, using only the crate's public API — no internal test-only helpers.

use smelt_core::config::{
    CellTechnique, MaintenanceCellConfig, MaintenanceDefaults, TechniquePreference,
};
use smelt_logical::maintenance::choice::{
    effective_override, resolve_cell_choice, ChosenTechnique, EffectiveOverride,
};
use smelt_logical::maintenance::{
    Corner, MaintenancePlan, PartitionLocal, PlanCell, RowIdentity, RowIdentityVerdict, Technique,
    Trigger,
};

/// A plan admitting exactly one `ColumnScopedMerge` cell for `source`'s
/// mutation trigger — the shape `derive_mutation` produces for a live
/// fact+dimension enrichment cell (mirrors
/// `crates/smelt-runtime/tests/technique_lowering.rs::admitted_plan`).
fn admitted_column_merge_plan(source: &str) -> MaintenancePlan {
    MaintenancePlan {
        cells: vec![PlanCell {
            group: "{user_name}".to_string(),
            trigger: Trigger::UpstreamMutation {
                source: source.to_string(),
            },
            corner: Corner::ColumnMerge,
            technique: Technique::ColumnScopedMerge,
            partition_local: PartitionLocal::Yes,
            scans: vec![],
            ledger_catch_up: false,
            row_identity: RowIdentityVerdict {
                identity: RowIdentity::WholeRow,
                proven_mismatch: None,
            },
            skeleton_source_closure: None,
            fingerprint_projections: std::collections::BTreeMap::new(),
        }],
        refusals: vec![],
        key_locality: None,
    }
}

/// `technique_pin_bypasses_cost_model_but_not_admission`: a `technique:` pin
/// bypasses the cost model (it changes the outcome relative to what
/// `resolve_cell_choice` would otherwise choose) but never bypasses
/// admission — pinning `fold` for a cell the derived plan only ever admitted
/// as a column-scoped merge is a hard [`ChoiceRefusal`], not a silent
/// override that lowers a technique the plan never proved safe.
#[test]
fn technique_pin_bypasses_cost_model_but_not_admission() {
    let plan = admitted_column_merge_plan("sources.raw.users");
    let trigger = Trigger::UpstreamMutation {
        source: "sources.raw.users".to_string(),
    };

    // The cost model's own default (no override at all) picks the admitted,
    // live technique.
    let no_override = EffectiveOverride::default();
    let default_choice = resolve_cell_choice(&plan, &trigger, &no_override, None, true)
        .expect("no override resolves without error");
    assert_eq!(
        default_choice,
        ChosenTechnique::Admitted(Technique::ColumnScopedMerge)
    );

    // A hard pin naming `recompute` bypasses that cost-model default,
    // changing the outcome to the always-admissible region recompute.
    let pin_recompute = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::Recompute),
    };
    let pinned_choice = resolve_cell_choice(&plan, &trigger, &pin_recompute, None, true)
        .expect("recompute is always in the resolvable set");
    assert_eq!(pinned_choice, ChosenTechnique::RegionRecompute);
    assert_ne!(
        pinned_choice, default_choice,
        "the pin must actually change the outcome — otherwise it isn't bypassing anything"
    );

    // But a pin naming `fold` — a technique this cell's plan never admitted
    // (only `ColumnScopedMerge` is admitted here) — is an error, not an
    // override: the pin bypasses the cost model, never admission.
    let pin_fold = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::Fold),
    };
    let err = resolve_cell_choice(&plan, &trigger, &pin_fold, None, true)
        .expect_err("pinning an unadmitted technique must refuse, never silently override");
    assert!(
        err.to_string().contains("MaintenanceUnboundedFootprint"),
        "refusal must name the diagnostic family: {err}"
    );

    // A pin naming the admitted technique itself when the backend cannot
    // run it (a capability gap) refuses the same way — a pin never silently
    // downgrades to a technique it didn't name.
    let pin_rederive = EffectiveOverride {
        prefer: None,
        technique: Some(CellTechnique::RederiveColumns),
    };
    let err2 = resolve_cell_choice(&plan, &trigger, &pin_rederive, None, false)
        .expect_err("pin naming a capability-gapped backend must refuse, not downgrade");
    assert!(err2.to_string().contains("MaintenanceUnboundedFootprint"));
}

/// `ladder_narrower_scope_wins`: `cells[].prefer`/`cells[].technique`
/// (narrower — scoped to one `(columns × trigger)` cell) must win over
/// `defaults.prefer` (broader — the whole model's soft default), and that
/// resolved override must actually change `resolve_cell_choice`'s outcome
/// relative to the broad default alone.
#[test]
fn ladder_narrower_scope_wins() {
    let defaults = MaintenanceDefaults {
        prefer: Some(TechniquePreference::Fold),
    };
    let narrow_cells = vec![MaintenanceCellConfig {
        columns: vec!["user_name".to_string()],
        on: "sources.raw.users".to_string(),
        prefer: Some(TechniquePreference::Recompute),
        technique: None,
        write: None,
    }];

    // Broad default alone (no matching cells[] entry): the model-wide
    // `prefer: fold` applies.
    let broad_only = effective_override(
        Some(&defaults),
        &[],
        "sources.raw.users",
        &["user_name".to_string()],
    );
    assert_eq!(broad_only.prefer, Some(TechniquePreference::Fold));

    // Narrower cells[] entry present: it wins over the broad default, even
    // though both are "soft" preferences.
    let narrowed = effective_override(
        Some(&defaults),
        &narrow_cells,
        "sources.raw.users",
        &["user_name".to_string()],
    );
    assert_eq!(narrowed.prefer, Some(TechniquePreference::Recompute));
    assert_ne!(
        narrowed.prefer, broad_only.prefer,
        "the narrower cells[] entry must actually change the effective preference"
    );

    // A `cells[].columns` entry naming a different cell entirely (a
    // different `on:`) must NOT shadow the broad default for this cell.
    let other_cell_cfg = vec![MaintenanceCellConfig {
        columns: vec!["other_col".to_string()],
        on: "sources.other".to_string(),
        prefer: Some(TechniquePreference::Recompute),
        technique: None,
        write: None,
    }];
    let unshadowed = effective_override(
        Some(&defaults),
        &other_cell_cfg,
        "sources.raw.users",
        &["user_name".to_string()],
    );
    assert_eq!(unshadowed.prefer, Some(TechniquePreference::Fold));

    // End-to-end: feeding the narrowed override into `resolve_cell_choice`
    // actually changes the resolved technique versus the broad default.
    let plan = admitted_column_merge_plan("sources.raw.users");
    let trigger = Trigger::UpstreamMutation {
        source: "sources.raw.users".to_string(),
    };
    let broad_choice = resolve_cell_choice(&plan, &trigger, &broad_only, None, true)
        .expect("soft prefer never refuses");
    let narrow_choice = resolve_cell_choice(&plan, &trigger, &narrowed, None, true)
        .expect("soft prefer never refuses");
    assert_eq!(
        broad_choice,
        ChosenTechnique::Admitted(Technique::ColumnScopedMerge)
    );
    assert_eq!(narrow_choice, ChosenTechnique::RegionRecompute);
}
