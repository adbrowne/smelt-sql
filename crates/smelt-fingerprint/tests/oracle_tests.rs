//! Phase 0 — the relation oracle itself, before any canonicaliser exists.
//!
//! These prove the soundness *measuring instrument* works: identical queries
//! compare equal, a projection reorder compares equal (matched by name), and a
//! genuine row difference compares unequal.

mod oracle;
use oracle::{relations_equal, DuckDbRelationOracle};

const SEED: &str = "WITH data AS (SELECT * FROM (VALUES \
    (1, 10.0), (2, 0.0), (3, 5.5)) t(id, total)) ";

#[test]
fn identical_queries_are_relation_equal() {
    let o = DuckDbRelationOracle::new();
    let q = format!("{SEED} SELECT id, total FROM data");
    let a = o.run(&q).unwrap();
    let b = o.run(&q).unwrap();
    relations_equal(&a, &b).unwrap();
}

#[test]
fn projection_reorder_is_relation_equal() {
    let o = DuckDbRelationOracle::new();
    let a = o
        .run(&format!("{SEED} SELECT id, total FROM data"))
        .unwrap();
    let b = o
        .run(&format!("{SEED} SELECT total, id FROM data"))
        .unwrap();
    // Same columns by name, same rows — equal despite different SELECT order.
    relations_equal(&a, &b).unwrap();
}

#[test]
fn different_rows_are_not_relation_equal() {
    let o = DuckDbRelationOracle::new();
    let a = o
        .run(&format!("{SEED} SELECT id, total FROM data"))
        .unwrap();
    // total * 2 genuinely changes the rows.
    let b = o
        .run(&format!("{SEED} SELECT id, total * 2 AS total FROM data"))
        .unwrap();
    assert!(
        relations_equal(&a, &b).is_err(),
        "expected differing relations to compare unequal"
    );
}

#[test]
fn different_column_names_are_not_relation_equal() {
    let o = DuckDbRelationOracle::new();
    let a = o
        .run(&format!("{SEED} SELECT id, total FROM data"))
        .unwrap();
    let b = o
        .run(&format!("{SEED} SELECT id, total AS amount FROM data"))
        .unwrap();
    assert!(
        relations_equal(&a, &b).is_err(),
        "expected differing column-name sets to compare unequal"
    );
}

#[test]
fn row_multiset_respects_duplicate_counts() {
    let o = DuckDbRelationOracle::new();
    // Two rows with the same value vs one — multiset must distinguish.
    let a = o.run("SELECT * FROM (VALUES (1), (1)) t(x)").unwrap();
    let b = o.run("SELECT * FROM (VALUES (1)) t(x)").unwrap();
    assert!(relations_equal(&a, &b).is_err());
}
