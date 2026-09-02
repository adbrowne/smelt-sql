//! `additive_only_diff`'s column-expression equality: a pure whitespace or
//! comment reformat of an *existing* column's expression must not be
//! classified as a semantic change. Phase 4 of
//! `docs/plans/20260808-substrate-unification.md` names this the single
//! sanctioned behaviour change — `additive_only_diff` moves off raw
//! `.text().trim()` comparison onto the same token-stream equality
//! `backbuild::diff` already uses (`same_modulo_trivia`).

use std::collections::BTreeSet;

use smelt_logical::analysis::model_diff::{additive_only_diff, ColumnDef, ModelDiff};
use smelt_logical::maintenance::derive::{derive_maintenance_plan, ModelInputs};
use smelt_logical::maintenance::{ColumnGroup, Corner, Grain, OutputSpec, Technique, Trigger};

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

#[test]
fn whitespace_reformat_is_not_a_change() {
    let old = vec![
        column_def("event_id", "event_id"),
        column_def("total", "amount +\n    tax"),
    ];
    let new = vec![
        column_def("event_id", "event_id"),
        // Same expression, reformatted: a line break plus a trailing
        // comment. Raw `.text().trim()` equality (the pre-fix
        // implementation) sees these as different strings and refuses the
        // edit as `NotAdditive`; token-stream equality (trivia-insensitive)
        // must see them as the same expression.
        column_def("total", "amount\n        + tax -- inline comment\n    "),
    ];

    let diff = additive_only_diff(&old, &new, &[]);
    assert_eq!(
        diff,
        ModelDiff::AdditiveOnly,
        "a pure whitespace/comment reformat must not be classified as a change: {diff:?}"
    );
}

#[test]
fn token_change_is_a_change() {
    let old = vec![column_def("total", "amount + tax")];
    let new = vec![column_def("total", "amount - tax")];

    let diff = additive_only_diff(&old, &new, &[]);
    assert!(
        matches!(diff, ModelDiff::NotAdditive { .. }),
        "a genuine one-token expression change must still be classified as a change: {diff:?}"
    );
}

// ---------------------------------------------------------------------------
// End-to-end leg: the fix threaded through `ModelInputs.column_add_proof`
// into `derive_maintenance_plan` (mirrors
// `maintenance_tracer.rs::ex36_pure_function_field_add_is_in_place_update_with_ledger_catch_up`).
//
// The model edit here reformats an *existing* column (`referrer`, unrelated
// to the add) with a line break and a trailing comment, alongside a genuine
// pure-function column add (`referrer_domain`). Under the pre-fix
// `.text().trim()` comparator, `referrer`'s reformat alone makes the whole
// diff `NotAdditive` (`old.expr.text().trim() != new_col.expr.text().trim()`
// still disagrees past a bare `.trim()` once an internal line break/comment
// is involved) — which refuses `referrer_domain`'s in-place admission too,
// since `column_add_proof` is one value shared by the whole `ColumnAdded`
// derivation. Post-fix, the reformat is a no-op and the genuine add is
// admitted exactly as EX-36 shows for a byte-identical add. This is the
// "derives no [spurious] `ColumnAdded` [refusal]" case named in the plan's
// Phase 4 review checklist: the reformat contributes no refusal cell.
// ---------------------------------------------------------------------------

#[test]
fn reformatted_unrelated_column_does_not_block_a_real_column_add() {
    let old = vec![
        column_def("event_id", "event_id"),
        column_def("referrer", "referrer"),
    ];
    let new = vec![
        column_def("event_id", "event_id"),
        // Same column, reformatted only — a line break plus a trailing
        // comment; no semantic change.
        column_def("referrer", "referrer\n        -- unchanged\n    "),
        column_def("referrer_domain", "SUBSTRING(referrer, 1, 10)"),
    ];

    let proof = additive_only_diff(&old, &new, &[]);
    assert_eq!(
        proof,
        ModelDiff::AdditiveOnly,
        "reformatting an unrelated existing column must not poison the add proof: {proof:?}"
    );

    let sql = "SELECT event_id, referrer, SUBSTRING(referrer, 1, 10) AS referrer_domain \
               FROM smelt.sources.events";
    let mut skeleton = BTreeSet::new();
    skeleton.insert("event_id".to_string());
    let inputs = ModelInputs {
        sql,
        output: OutputSpec {
            table: "events_enriched".to_string(),
            grain: Grain::Key {
                unique_key: vec!["event_id".to_string()],
            },
            skeleton_columns: skeleton,
        },
        sources: vec![],
        column_groups: vec![ColumnGroup {
            columns: vec!["referrer_domain".to_string()],
            mutation_sensitivity: BTreeSet::new(),
            membership_sensitivity: BTreeSet::new(),
        }],
        fold: None,
        old_columns: old,
        old_sql: None,
    };

    let plan = derive_maintenance_plan(
        &inputs,
        &[Trigger::ColumnAdded {
            columns: vec!["referrer_domain".to_string()],
        }],
    );

    assert!(
        plan.refusals.is_empty(),
        "the reformat must not spuriously refuse the real column add: {:?}",
        plan.refusals
    );
    assert_eq!(plan.cells.len(), 1, "cells: {:?}", plan.cells);
    assert_eq!(plan.cells[0].corner, Corner::FoldDelta);
    assert_eq!(plan.cells[0].technique, Technique::InPlaceUpdate);
}
