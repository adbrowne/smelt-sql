//! TDD tests for the `MaintenanceColumnAddNotBackfillable` posture
//! (`docs/specs/definition_deltas.md` §"Detection"): a non-skeleton
//! `Trigger::ColumnAdded` that cannot be backfilled in place derives
//! `Refusal::DefinitionChangeNotBackfillable`, never
//! `Refusal::ScanUnbounded`/`Refusal::NoAdmissibleTechnique` — those codes
//! stay reserved for an ordinary fold refusal, which a run genuinely
//! refuses (unlike a non-backfillable column add, which a run admits with a
//! warning).

use std::collections::BTreeSet;

use smelt_logical::analysis::model_diff::ColumnDef;
use smelt_logical::maintenance::derive::{derive_maintenance_plan, ModelInputs};
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MutationProfile, OutputSpec, Refusal, SourceFacts, Trigger,
};

fn column_def(name: &str, sql_expr: &str) -> ColumnDef {
    let sql = format!("SELECT {sql_expr} AS v FROM t");
    let parse = smelt_parser::parse(&sql);
    let file = smelt_parser::File::cast(parse.syntax()).expect("file");
    let select = file.select_stmt().expect("select");
    let item = select
        .select_list()
        .expect("select list")
        .items()
        .next()
        .expect("item");
    ColumnDef {
        name: name.to_string(),
        expr: item.expression().expect("expression"),
    }
}

fn old_columns() -> Vec<ColumnDef> {
    vec![column_def("id", "base.id"), column_def("a", "base.a")]
}

/// A column-scoped merge whose only mutation-sensitive source is unclocked
/// and not declared `allow_full_scan` — the backfill's scan cannot be
/// partition-bounded. Reported as `DefinitionChangeNotBackfillable`, not
/// `ScanUnbounded` (a run still admits and ALTERs the column in).
#[test]
fn column_add_with_unbounded_merge_source_refuses_not_backfillable() {
    let sql = "SELECT base.id, ext.d AS d FROM smelt.sources.base base \
               JOIN smelt.sources.ext ext ON ext.id = base.id";
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "t".to_string(),
            grain: Grain::Key {
                unique_key: vec!["id".to_string()],
            },
            skeleton_columns: BTreeSet::from(["id".to_string()]),
        },
        sources: vec![
            SourceFacts {
                name: "base".to_string(),
                mutation: MutationProfile::AppendOnly,
                partition_col: None,
                unique_key: vec![],
                allow_full_scan: true,
            },
            SourceFacts {
                name: "ext".to_string(),
                mutation: MutationProfile::AppendOnly,
                partition_col: None,
                unique_key: vec![],
                allow_full_scan: false,
            },
        ],
        column_groups: vec![ColumnGroup {
            columns: vec!["d".to_string()],
            mutation_sensitivity: BTreeSet::from(["ext".to_string()]),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: None,
        old_columns: old_columns(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: vec!["d".to_string()],
        }],
    );
    assert!(plan.cells.is_empty(), "cells: {:?}", plan.cells);
    match &plan.refusals[..] {
        [Refusal::DefinitionChangeNotBackfillable { columns, why }] => {
            assert_eq!(columns, &["d".to_string()]);
            assert!(why.contains("no partition bound"), "why: {why}");
        }
        other => panic!("expected a single DefinitionChangeNotBackfillable refusal, got {other:?}"),
    }
}

/// The empty-mutation-sensitivity path with no admissible technique — the
/// added column's expression cannot be resolved in the model's own SQL —
/// also reports `DefinitionChangeNotBackfillable`, not
/// `NoAdmissibleTechnique`.
#[test]
fn column_add_not_proven_additive_refuses_not_backfillable() {
    // `d` is not actually present in the SELECT list, so
    // `column_def_from_sql` cannot resolve its expression.
    let sql = "SELECT base.id, base.a FROM smelt.sources.base base";
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "t".to_string(),
            grain: Grain::Key {
                unique_key: vec!["id".to_string()],
            },
            skeleton_columns: BTreeSet::from(["id".to_string()]),
        },
        sources: vec![SourceFacts {
            name: "base".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: true,
        }],
        column_groups: vec![ColumnGroup {
            columns: vec!["d".to_string()],
            mutation_sensitivity: BTreeSet::new(),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: None,
        old_columns: old_columns(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: vec!["d".to_string()],
        }],
    );
    assert!(plan.cells.is_empty(), "cells: {:?}", plan.cells);
    match &plan.refusals[..] {
        [Refusal::DefinitionChangeNotBackfillable { columns, .. }] => {
            assert_eq!(columns, &["d".to_string()]);
        }
        other => panic!("expected a single DefinitionChangeNotBackfillable refusal, got {other:?}"),
    }
}

/// A skeleton-position add (a declared `output.skeleton_columns` member)
/// keeps refusing `SkeletonChanged` — unaffected by the not-backfillable
/// posture, since a grain change is never admitted with a warning.
#[test]
fn skeleton_position_column_add_still_refuses_skeleton_changed() {
    let sql = "SELECT base.id, base.grp AS grp, SUM(base.a) AS total \
               FROM smelt.sources.base base GROUP BY base.id, base.grp";
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "t".to_string(),
            grain: Grain::Key {
                unique_key: vec!["id".to_string()],
            },
            skeleton_columns: BTreeSet::from(["id".to_string(), "grp".to_string()]),
        },
        sources: vec![SourceFacts {
            name: "base".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: true,
        }],
        column_groups: vec![ColumnGroup {
            columns: vec!["grp".to_string()],
            mutation_sensitivity: BTreeSet::new(),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: None,
        old_columns: old_columns(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: vec!["grp".to_string()],
        }],
    );
    assert!(plan.cells.is_empty(), "cells: {:?}", plan.cells);
    match &plan.refusals[..] {
        [Refusal::SkeletonChanged { column }] => assert_eq!(column, "grp"),
        other => panic!("expected a single SkeletonChanged refusal, got {other:?}"),
    }
}

/// No regression: a pure function of already-stored columns still admits
/// the cheap in-place `UPDATE` technique, no refusal at all.
#[test]
fn admitted_pure_backfill_column_add_still_yields_in_place_update_cell() {
    let sql = "SELECT base.id, base.a + 1 AS c FROM smelt.sources.base base";
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "t".to_string(),
            grain: Grain::Key {
                unique_key: vec!["id".to_string()],
            },
            skeleton_columns: BTreeSet::from(["id".to_string()]),
        },
        sources: vec![SourceFacts {
            name: "base".to_string(),
            mutation: MutationProfile::AppendOnly,
            partition_col: None,
            unique_key: vec![],
            allow_full_scan: true,
        }],
        column_groups: vec![ColumnGroup {
            columns: vec!["c".to_string()],
            mutation_sensitivity: BTreeSet::new(),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: None,
        old_columns: old_columns(),
        old_sql: None,
        keyed_time_axis: None,
        old_partition_col: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: vec!["c".to_string()],
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    assert_eq!(plan.cells.len(), 1, "cells: {:?}", plan.cells);
}
