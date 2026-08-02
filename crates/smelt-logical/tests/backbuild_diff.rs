//! Diff-foundation tests for `smelt_logical::backbuild::definition_diff`.
//!
//! See `docs/research/20260802-backbuild-synthesis.md` §3, §4 (case A0), and
//! §6 (`diff.rs`), and `.superpowers/sdd/20260802-backbuild-synthesis/task-1-brief.md`.

use smelt_logical::backbuild::{
    definition_diff, ConjunctDiff, DefinitionDiff, SelectListDiff, SetOpDiff, SkeletonDiff,
};
use smelt_parser::JoinType;

fn parse(sql: &str) -> smelt_parser::File {
    let parse = smelt_parser::parse(sql);
    smelt_parser::File::cast(parse.syntax()).expect("file")
}

fn comparable(diff: &DefinitionDiff) -> &smelt_logical::backbuild::ComparableDiff {
    match diff {
        DefinitionDiff::Comparable(c) => c,
        DefinitionDiff::Opaque { reason } => panic!("expected Comparable, got Opaque: {reason}"),
    }
}

#[test]
fn a0_formatting_only_is_noop() {
    let before = parse(
        "SELECT id, amount FROM orders WHERE amount > 0 -- keep positive orders\nORDER BY id",
    );
    let after = parse(
        "SELECT\n  id,\n  amount\nFROM orders\nWHERE amount > 0  -- keep positive orders\nORDER BY id",
    );

    let diff = definition_diff(&before, &after);
    assert!(
        diff.is_noop(),
        "whitespace/comment-only reformat should be a no-op: {diff:?}"
    );
}

#[test]
fn added_column_detected() {
    let before = parse("SELECT id, amount FROM orders");
    let after = parse("SELECT id, amount, amount * 2 AS doubled FROM orders");

    let diff = definition_diff(&before, &after);
    let c = comparable(&diff);
    match &c.select_list {
        SelectListDiff::Diffed {
            added,
            unchanged,
            dropped,
            changed,
        } => {
            assert_eq!(dropped.len(), 0);
            assert_eq!(changed.len(), 0);
            assert_eq!(added.len(), 1);
            assert_eq!(added[0].name, "doubled");
            assert_eq!(added[0].expr.text().trim(), "amount * 2");

            let unchanged_names: Vec<&str> = unchanged.iter().map(|c| c.name.as_str()).collect();
            assert_eq!(unchanged_names.len(), 2);
            assert!(unchanged_names.contains(&"id"));
            assert!(unchanged_names.contains(&"amount"));
        }
        other => panic!("expected Diffed select_list, got {other:?}"),
    }
    assert!(!diff.is_noop());
}

#[test]
fn dropped_and_changed_columns_detected() {
    let before = parse("SELECT id, amount, legacy_flag FROM orders");
    let after = parse("SELECT id, amount * 2 AS amount FROM orders");

    let diff = definition_diff(&before, &after);
    let c = comparable(&diff);
    match &c.select_list {
        SelectListDiff::Diffed {
            added,
            dropped,
            changed,
            unchanged,
        } => {
            assert_eq!(added.len(), 0);
            assert_eq!(dropped.len(), 1);
            assert_eq!(dropped[0].name, "legacy_flag");

            assert_eq!(changed.len(), 1);
            assert_eq!(changed[0].name, "amount");
            assert_eq!(changed[0].before.text().trim(), "amount");
            assert_eq!(changed[0].after.text().trim(), "amount * 2");

            let unchanged_names: Vec<&str> = unchanged.iter().map(|c| c.name.as_str()).collect();
            assert_eq!(unchanged_names, vec!["id"]);
        }
        other => panic!("expected Diffed select_list, got {other:?}"),
    }
}

#[test]
fn expression_change_is_trivia_insensitive() {
    // Reformatting one expression (whitespace only) must not report a change.
    let before = parse("SELECT id, amount*2 AS doubled FROM orders");
    let after = parse("SELECT id, amount  *  2 AS doubled FROM orders");

    let diff = definition_diff(&before, &after);
    let c = comparable(&diff);
    match &c.select_list {
        SelectListDiff::Diffed {
            changed, unchanged, ..
        } => {
            assert_eq!(
                changed.len(),
                0,
                "whitespace-only reformat must not be a change"
            );
            let unchanged_names: Vec<&str> = unchanged.iter().map(|c| c.name.as_str()).collect();
            assert!(unchanged_names.contains(&"doubled"));
        }
        other => panic!("expected Diffed select_list, got {other:?}"),
    }

    // An actual token change (2 -> 3) must be reported.
    let after_changed = parse("SELECT id, amount * 3 AS doubled FROM orders");
    let diff2 = definition_diff(&before, &after_changed);
    let c2 = comparable(&diff2);
    match &c2.select_list {
        SelectListDiff::Diffed { changed, .. } => {
            assert_eq!(changed.len(), 1);
            assert_eq!(changed[0].name, "doubled");
        }
        other => panic!("expected Diffed select_list, got {other:?}"),
    }
}

#[test]
fn where_conjunct_diff_added_and_removed() {
    let before = parse("SELECT id FROM orders WHERE a > 1 AND b > 2");
    let after = parse("SELECT id FROM orders WHERE a > 1 AND c > 3");

    let diff = definition_diff(&before, &after);
    let c = comparable(&diff);
    match &c.where_clause {
        ConjunctDiff::Diffed {
            added,
            removed,
            unchanged,
        } => {
            assert_eq!(removed.len(), 1);
            assert_eq!(removed[0].text().trim(), "b > 2");
            assert_eq!(added.len(), 1);
            assert_eq!(added[0].text().trim(), "c > 3");
            assert_eq!(unchanged.len(), 1);
            assert_eq!(unchanged[0].text().trim(), "a > 1");
        }
        other => panic!("expected Diffed where_clause, got {other:?}"),
    }

    // A non-conjunctive rewrite (a OR b -> a) must refuse rather than
    // report a member swap.
    let or_before = parse("SELECT id FROM orders WHERE a OR b");
    let or_after = parse("SELECT id FROM orders WHERE a");
    let or_diff = definition_diff(&or_before, &or_after);
    let or_c = comparable(&or_diff);
    assert!(
        matches!(or_c.where_clause, ConjunctDiff::Opaque { .. }),
        "expected Opaque where_clause diff for `a OR b` -> `a`, got {:?}",
        or_c.where_clause
    );
}

#[test]
fn skeleton_join_add_detected() {
    let before = parse("SELECT o.id FROM orders o JOIN customers c ON o.customer_id = c.id");
    let after = parse(
        "SELECT o.id FROM orders o JOIN customers c ON o.customer_id = c.id \
         LEFT JOIN regions r ON c.region_id = r.id",
    );

    let diff = definition_diff(&before, &after);
    let c = comparable(&diff);
    match &c.skeleton {
        SkeletonDiff::AddedLeftJoins(added) => {
            assert_eq!(added.len(), 1);
            assert_eq!(added[0].join_type(), Some(JoinType::Left));
        }
        other => panic!("expected AddedLeftJoins, got {other:?}"),
    }

    // A changed join condition (otherwise same shape) must be Changed, not
    // AddedLeftJoins.
    let after_changed_cond =
        parse("SELECT o.id FROM orders o JOIN customers c ON o.customer_id = c.legacy_id");
    let diff2 = definition_diff(&before, &after_changed_cond);
    let c2 = comparable(&diff2);
    assert!(
        matches!(c2.skeleton, SkeletonDiff::Changed { .. }),
        "expected Changed skeleton diff for an edited join condition, got {:?}",
        c2.skeleton
    );
}

#[test]
fn union_branch_diff() {
    let before = parse("SELECT a FROM t1 UNION ALL SELECT a FROM t2");
    // Reordered + one added branch.
    let after = parse("SELECT a FROM t2 UNION ALL SELECT a FROM t1 UNION ALL SELECT a FROM t3");

    let diff = definition_diff(&before, &after);
    let c = comparable(&diff);
    match &c.set_ops {
        SetOpDiff::Branches {
            added,
            removed,
            unchanged,
        } => {
            assert_eq!(
                removed.len(),
                0,
                "reordered identical branches must not be removed"
            );
            assert_eq!(unchanged.len(), 2);
            assert_eq!(added.len(), 1);
        }
        other => panic!("expected Branches set_ops diff, got {other:?}"),
    }
}

#[test]
fn changed_cte_is_opaque() {
    let before = parse("WITH base AS (SELECT id, amount FROM orders) SELECT id, amount FROM base");
    let after_changed_cte = parse(
        "WITH base AS (SELECT id, amount * 2 AS amount FROM orders) SELECT id, amount FROM base",
    );

    let diff = definition_diff(&before, &after_changed_cte);
    assert!(
        matches!(diff, DefinitionDiff::Opaque { .. }),
        "an edit inside a CTE body must be opaque, got {diff:?}"
    );
    assert!(!diff.is_noop());

    // An unchanged WITH prefix with an edited final SELECT diffs normally.
    let after_final_edited = parse(
        "WITH base AS (SELECT id, amount FROM orders) SELECT id, amount, amount * 2 AS doubled FROM base",
    );
    let diff2 = definition_diff(&before, &after_final_edited);
    let c2 = comparable(&diff2);
    match &c2.select_list {
        SelectListDiff::Diffed { added, .. } => {
            assert_eq!(added.len(), 1);
            assert_eq!(added[0].name, "doubled");
        }
        other => panic!("expected Diffed select_list, got {other:?}"),
    }
}
