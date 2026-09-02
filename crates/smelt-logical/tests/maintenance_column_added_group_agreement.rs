//! TDD tests for `derive_column_added`'s group-agreement check
//! (`smelt_logical::maintenance::derive`): when a `Trigger::ColumnAdded`
//! names more than one added column in the same column group, every added
//! column's own `classify_definition_change` verdict
//! (`model_properties.md` §"Definition-change column classification") must
//! agree — a group is maintained by ONE technique, so two added columns
//! that would need different techniques (an in-place `UPDATE` for a pure
//! backfill vs. a column-scoped `MERGE` for an upstream re-derive) cannot
//! both be served by this trigger's single cell.

use std::collections::BTreeSet;

use smelt_logical::analysis::model_diff::ColumnDef;
use smelt_logical::maintenance::derive::{derive_maintenance_plan, group_columns, ModelInputs};
use smelt_logical::maintenance::{
    ColumnGroup, Grain, MutationProfile, OutputSpec, Refusal, RowIdentity, SourceFacts, Trigger,
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
    vec![
        column_def("id", "base.id"),
        column_def("a", "base.a"),
        column_def("b", "base.b"),
    ]
}

/// Two added columns in the SAME group, one a pure function of already-
/// stored columns (`PureBackfill`) and the other reading an upstream source
/// column never before stored (`UpstreamRederive`) — the group's verdicts
/// disagree, so the trigger has no single admissible technique and must
/// refuse rather than pick one arbitrarily.
#[test]
fn disagreeing_added_columns_in_one_group_refuse() {
    let sql = "SELECT base.id, base.a, base.b, base.a + base.b AS c, ext.d AS d \
               FROM smelt.sources.base base \
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
                allow_full_scan: true,
            },
        ],
        column_groups: vec![ColumnGroup {
            columns: vec!["c".to_string(), "d".to_string()],
            mutation_sensitivity: BTreeSet::new(),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: None,
        old_columns: old_columns(),
        old_sql: None,
        keyed_time_axis: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: vec!["c".to_string(), "d".to_string()],
        }],
    );
    assert!(plan.cells.is_empty(), "cells: {:?}", plan.cells);
    assert!(
        matches!(&plan.refusals[..], [Refusal::DefinitionChangeNotBackfillable { why, .. }]
            if why.contains("disagree")),
        "expected a group-disagreement DefinitionChangeNotBackfillable refusal, got {:?}",
        plan.refusals
    );
}

/// Two added columns in the SAME group that BOTH classify `PureBackfill`
/// (pure functions of already-stored columns) agree — the group admits the
/// single in-place-update technique, exercising the "agree" arm of the same
/// comparison the disagreement test exercises the "disagree" arm of.
#[test]
fn agreeing_added_columns_in_one_group_admit_the_shared_technique() {
    let sql =
        "SELECT base.id, base.a, base.b, base.a + 1 AS c, base.b + 1 AS d FROM smelt.sources.base base";
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
            columns: vec!["c".to_string(), "d".to_string()],
            mutation_sensitivity: BTreeSet::new(),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: None,
        old_columns: old_columns(),
        old_sql: None,
        keyed_time_axis: None,
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: vec!["c".to_string(), "d".to_string()],
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    assert_eq!(plan.cells.len(), 1, "cells: {:?}", plan.cells);
}

/// `group_columns` flattens the column name set across every group — a
/// convenience callers (e.g. a downstream `Trigger::ColumnAdded` builder)
/// use to compute "every column any group governs" without re-deriving the
/// flattening themselves. Two groups, overlapping and non-overlapping
/// columns, pins the flatten-and-dedupe (`BTreeSet`) shape precisely.
#[test]
fn group_columns_flattens_and_dedupes_across_groups() {
    let groups = vec![
        ColumnGroup {
            columns: vec!["a".to_string(), "b".to_string()],
            mutation_sensitivity: BTreeSet::new(),
            membership_sensitivity: BTreeSet::new(),
        },
        ColumnGroup {
            columns: vec!["b".to_string(), "c".to_string()],
            mutation_sensitivity: BTreeSet::new(),
            membership_sensitivity: BTreeSet::new(),
        },
    ];
    let expected: BTreeSet<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
    assert_eq!(group_columns(&groups), expected);
}

/// `ModelInputs::declared_unique_key` — `Grain::Key`'s own declared
/// `unique_key`, threaded into `row_identity` — must carry the EXACT
/// declared columns through to every cell's `row_identity`, not a stand-in
/// value. No existing `Grain::Key` fixture asserted the derived plan's
/// `cell.row_identity` content precisely; this pins it directly.
#[test]
fn declared_unique_key_carries_through_to_every_cells_row_identity() {
    let sql = "SELECT base.id, base.region, base.a + 1 AS c FROM smelt.sources.base base";
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "t".to_string(),
            grain: Grain::Key {
                unique_key: vec!["id".to_string(), "region".to_string()],
            },
            skeleton_columns: BTreeSet::from(["id".to_string(), "region".to_string()]),
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
    };
    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: vec!["c".to_string()],
        }],
    );
    assert!(plan.refusals.is_empty(), "refusals: {:?}", plan.refusals);
    assert_eq!(
        plan.cells[0].row_identity.identity,
        RowIdentity::Key(vec!["id".to_string(), "region".to_string()]),
        "the cell's row_identity must carry the exact declared unique_key, got {:?}",
        plan.cells[0].row_identity
    );
}
