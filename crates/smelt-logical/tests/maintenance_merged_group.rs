//! Phase 28b (`docs/outcomes/20260815-definition-delta-migrate`): the
//! group-merge-provenance decision (`incremental_models.md` §"The plan
//! matrix") pinned — a column group whose sensitivity spans two or more
//! mutation-capable inputs is repaired by region recompute, never a
//! column-scoped merge, even when neither input alone is membership-
//! sensitive. Conservative, always-correct default: a value change that
//! could in principle be a mixed contribution from two independently
//! mutating sources is never assumed safe to `MERGE` column-scoped.

use std::collections::BTreeSet;

use smelt_logical::maintenance::derive::{derive_maintenance_plan, ModelInputs};
use smelt_logical::maintenance::{
    ColumnGroup, Corner, Grain, MutationProfile, OutputSpec, SourceFacts, Technique, Trigger,
};

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn mutable_source(name: &str) -> SourceFacts {
    SourceFacts {
        name: name.to_string(),
        mutation: MutationProfile::MutableSnapshot,
        partition_col: None,
        unique_key: vec![],
        allow_full_scan: true,
    }
}

fn append_only_source(name: &str) -> SourceFacts {
    SourceFacts {
        name: name.to_string(),
        mutation: MutationProfile::AppendOnly,
        partition_col: None,
        unique_key: vec![],
        allow_full_scan: true,
    }
}

fn base_inputs(sources: Vec<SourceFacts>, column_groups: Vec<ColumnGroup>) -> ModelInputs<'static> {
    ModelInputs {
        sql: "SELECT id, amount FROM smelt.sources.a JOIN smelt.sources.b USING (id)",
        output: OutputSpec {
            table: "merged".to_string(),
            grain: Grain::Key {
                unique_key: strings(&["id"]),
            },
            skeleton_columns: set(&["id"]),
        },
        sources,
        column_groups,
        fold: None,
        old_columns: Vec::new(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    }
}

/// A column group value-sensitive to TWO mutation-capable inputs takes
/// region recompute for each source's `UpstreamMutation` trigger — no
/// `ColumnScopedMerge` cell exists for that group.
#[test]
fn merged_group_takes_region_recompute() {
    let sources = vec![mutable_source("a"), mutable_source("b")];
    let column_groups = vec![ColumnGroup {
        columns: strings(&["amount"]),
        mutation_sensitivity: set(&["a", "b"]),
        membership_sensitivity: BTreeSet::new(),
    }];
    let inputs = base_inputs(sources, column_groups);
    let triggers = vec![
        Trigger::UpstreamMutation {
            source: "a".to_string(),
        },
        Trigger::UpstreamMutation {
            source: "b".to_string(),
        },
    ];
    let plan = derive_maintenance_plan(&inputs, &triggers);

    assert_eq!(
        plan.cells.len(),
        2,
        "expected one cell per trigger, got {:?}",
        plan.cells
    );
    for cell in &plan.cells {
        assert_eq!(
            cell.corner,
            Corner::RecomputeRegion,
            "merged group must take region recompute, got {:?}",
            cell
        );
        assert_eq!(
            cell.technique,
            Technique::DeleteInsert,
            "merged group must take DeleteInsert, got {:?}",
            cell
        );
    }
    assert!(
        !plan
            .cells
            .iter()
            .any(|c| c.technique == Technique::ColumnScopedMerge),
        "no ColumnScopedMerge cell should exist for a merged group, got {:?}",
        plan.cells
    );
}

/// Control: the same shape with only ONE mutable input still admits the
/// cheaper column-scoped merge — the guard is scoped to genuinely merged
/// groups, not a blanket downgrade of every value-sensitive group.
#[test]
fn single_mutable_input_group_keeps_the_column_merge() {
    let sources = vec![mutable_source("a")];
    let column_groups = vec![ColumnGroup {
        columns: strings(&["amount"]),
        mutation_sensitivity: set(&["a"]),
        membership_sensitivity: BTreeSet::new(),
    }];
    let inputs = base_inputs(sources, column_groups);
    let triggers = vec![Trigger::UpstreamMutation {
        source: "a".to_string(),
    }];
    let plan = derive_maintenance_plan(&inputs, &triggers);

    assert_eq!(
        plan.cells.len(),
        1,
        "expected one cell, got {:?}",
        plan.cells
    );
    assert_eq!(plan.cells[0].corner, Corner::ColumnMerge);
    assert_eq!(plan.cells[0].technique, Technique::ColumnScopedMerge);
}

/// A group sensitive to one mutable source plus one append-only source is
/// NOT merged for this rule's purpose (the append-only source is not
/// mutation-capable) — the group keeps the column-scoped merge.
#[test]
fn merged_group_rule_counts_only_mutation_capable_inputs() {
    let sources = vec![mutable_source("a"), append_only_source("events")];
    let column_groups = vec![ColumnGroup {
        columns: strings(&["amount"]),
        mutation_sensitivity: set(&["a", "events"]),
        membership_sensitivity: BTreeSet::new(),
    }];
    let inputs = base_inputs(sources, column_groups);
    // `events` is append-only and not value-sensitive-triggered here (no
    // separate mutation trigger derived for it in this fixture) — only `a`
    // gets an UpstreamMutation trigger.
    let triggers = vec![Trigger::UpstreamMutation {
        source: "a".to_string(),
    }];
    let plan = derive_maintenance_plan(&inputs, &triggers);

    assert_eq!(
        plan.cells.len(),
        1,
        "expected one cell, got {:?}",
        plan.cells
    );
    assert_eq!(plan.cells[0].corner, Corner::ColumnMerge);
    assert_eq!(plan.cells[0].technique, Technique::ColumnScopedMerge);
}

/// Regression guard: the existing membership-sensitivity branch (a single
/// mutation-capable input, membership-sensitive) is unaffected by the new
/// guard's placement — it still forces region recompute for the reason it
/// always did.
#[test]
fn membership_sensitivity_still_forces_recompute_for_a_single_input() {
    let sources = vec![mutable_source("a")];
    let column_groups = vec![ColumnGroup {
        columns: strings(&["amount"]),
        mutation_sensitivity: BTreeSet::new(),
        membership_sensitivity: set(&["a"]),
    }];
    let inputs = base_inputs(sources, column_groups);
    let triggers = vec![Trigger::UpstreamMutation {
        source: "a".to_string(),
    }];
    let plan = derive_maintenance_plan(&inputs, &triggers);

    assert_eq!(
        plan.cells.len(),
        1,
        "expected one cell, got {:?}",
        plan.cells
    );
    assert_eq!(plan.cells[0].corner, Corner::RecomputeRegion);
    assert_eq!(plan.cells[0].technique, Technique::DeleteInsert);
}
