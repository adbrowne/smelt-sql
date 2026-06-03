//! Positive corpus — §5.3 #1 refactors that must be recognised as equivalent.
//!
//! Each case asserts BOTH that smelt's fingerprint is equal AND that DuckDB
//! confirms the two queries produce the same relation. The fingerprint equality
//! is the claim; the DuckDB check is the soundness witness for that specific
//! pair.
//!
//! Cases are added as each canonicalisation rule lands (see the plan's phased
//! breakdown). Phase 1: formatting + comments.

mod oracle;
use oracle::{relations_equal, DuckDbRelationOracle};
use smelt_fingerprint::output_fingerprint_from_sql;

/// Assert two query texts are fingerprint-equal and DuckDB-relation-equal.
#[track_caller]
fn assert_equivalent(a: &str, b: &str) {
    let fa = output_fingerprint_from_sql(a, &[]).expect("a parses to a SELECT");
    let fb = output_fingerprint_from_sql(b, &[]).expect("b parses to a SELECT");
    assert_eq!(
        fa.fingerprint, fb.fingerprint,
        "fingerprints differ but should match:\n  A: {a}\n  B: {b}"
    );

    let o = DuckDbRelationOracle::new();
    let ra = o
        .run(a)
        .unwrap_or_else(|e| panic!("A failed to run: {e}\n{a}"));
    let rb = o
        .run(b)
        .unwrap_or_else(|e| panic!("B failed to run: {e}\n{b}"));
    relations_equal(&ra, &rb)
        .unwrap_or_else(|e| panic!("DuckDB says relations differ ({e}):\n  A: {a}\n  B: {b}"));
}

// Column names come from a `(VALUES …) AS t(id, total)` column-alias list —
// the real DuckDB syntax, now that smelt's parser captures it (see
// `docs/specs/types.md` §"VALUES-derived tables").
const SEED: &str = "WITH data AS (\
    SELECT id, total \
    FROM (VALUES (1, 10.0), (2, 0.0), (3, 5.5)) AS t(id, total))";

#[test]
fn whitespace_reformatting_is_equivalent() {
    let a = format!("{SEED} SELECT id, total FROM data WHERE total > 0");
    let b = format!("{SEED}\n   SELECT   id,\n          total\n   FROM     data\n   WHERE total>0");
    assert_equivalent(&a, &b);
}

#[test]
fn line_comment_is_ignored() {
    let a = format!("{SEED} SELECT id, total FROM data");
    let b = format!("{SEED} SELECT id, -- the identifier\n total FROM data");
    assert_equivalent(&a, &b);
}

#[test]
fn block_comment_is_ignored() {
    let a = format!("{SEED} SELECT id, total FROM data");
    let b = format!("{SEED} SELECT id, total /* trailing note */ FROM data");
    assert_equivalent(&a, &b);
}

// ---- Phase 2: projection reordering ----

#[test]
fn projection_reorder_is_equivalent() {
    let a = format!("{SEED} SELECT id, total FROM data");
    let b = format!("{SEED} SELECT total, id FROM data");
    assert_equivalent(&a, &b);
}

#[test]
fn projection_reorder_with_aliases_is_equivalent() {
    let a = format!("{SEED} SELECT id AS order_id, total AS revenue FROM data");
    let b = format!("{SEED} SELECT total AS revenue, id AS order_id FROM data");
    assert_equivalent(&a, &b);
}

// ---- Phase 3: keyword case is insensitive (SQL keywords are case-insensitive) ----

#[test]
fn keyword_case_is_equivalent() {
    let a = format!("{SEED} SELECT id, total FROM data WHERE total > 0");
    let b = format!("{SEED} select id, total from data where total > 0");
    assert_equivalent(&a, &b);
}

// ---- Phase 4: internal alias / CTE name renaming ----

#[test]
fn cte_name_rename_is_equivalent() {
    let a = "WITH the_orders AS (SELECT 1 AS id, 10.0 AS total) \
             SELECT id, total FROM the_orders";
    let b = "WITH x AS (SELECT 1 AS id, 10.0 AS total) \
             SELECT id, total FROM x";
    assert_equivalent(a, b);
}

#[test]
fn cte_name_rename_with_qualifier_is_equivalent() {
    let a = "WITH the_orders AS (SELECT 1 AS id, 10.0 AS total) \
             SELECT the_orders.id, the_orders.total FROM the_orders";
    let b = "WITH q AS (SELECT 1 AS id, 10.0 AS total) \
             SELECT q.id, q.total FROM q";
    assert_equivalent(a, b);
}

#[test]
fn derived_table_alias_rename_is_equivalent() {
    let a = "SELECT foo.id FROM (SELECT 1 AS id) AS foo";
    let b = "SELECT bar.id FROM (SELECT 1 AS id) AS bar";
    assert_equivalent(a, b);
}

#[test]
fn column_named_like_alias_is_not_renamed() {
    // `total` is a column, and we use a CTE also exercising the alias map; the
    // column must keep its identity (not be rewritten to a positional name),
    // otherwise the relation's column-name set would change.
    let a = "WITH d AS (SELECT 1 AS id, 10.0 AS total) SELECT total FROM d";
    let b = "WITH e AS (SELECT 1 AS id, 10.0 AS total) SELECT total FROM e";
    assert_equivalent(a, b);
}

// ---- Phase 5: single-use CTE inline ≡ derived table ----

#[test]
fn single_use_cte_equals_derived_table() {
    // A CTE referenced exactly once is semantically a derived table.
    let a = "WITH c AS (SELECT 1 AS id) SELECT c.id FROM c";
    let b = "SELECT t.id FROM (SELECT 1 AS id) AS t";
    assert_equivalent(a, b);
}

#[test]
fn cte_inline_multi_column() {
    let a = "WITH c AS (SELECT 1 AS id, 2 AS amt) SELECT c.id, c.amt FROM c";
    let b = "SELECT t.amt, t.id FROM (SELECT 1 AS id, 2 AS amt) AS t";
    assert_equivalent(a, b);
}

#[test]
fn cte_inline_with_inner_refactor() {
    // The inlined subquery is itself fingerprinted recursively, so a refactor
    // inside the CTE body (projection reorder) is still recognised as equivalent.
    let a = "WITH c AS (SELECT 1 AS id, 2 AS amt) SELECT c.id, c.amt FROM c";
    let b = "WITH other AS (SELECT 2 AS amt, 1 AS id) SELECT other.id, other.amt FROM other";
    assert_equivalent(a, b);
}
