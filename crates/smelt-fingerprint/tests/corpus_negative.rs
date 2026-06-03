//! Negative corpus — changes that genuinely alter output must change the
//! fingerprint.
//!
//! These do NOT assert that DuckDB rows differ (two semantically different
//! queries can coincidentally agree on a particular seed). The claim is purely:
//! a real change must move the fingerprint. A change that left the fingerprint
//! equal here would be a soundness bug (a false "equivalent").

use smelt_fingerprint::output_fingerprint_from_sql;

const SEED: &str = "WITH data AS (\
    SELECT 1 AS id, 10.0 AS total UNION ALL \
    SELECT 2, 0.0 UNION ALL \
    SELECT 3, 5.5)";

#[track_caller]
fn assert_distinct(a: &str, b: &str) {
    let fa = output_fingerprint_from_sql(a, &[]).expect("a parses");
    let fb = output_fingerprint_from_sql(b, &[]).expect("b parses");
    assert_ne!(
        fa.fingerprint, fb.fingerprint,
        "fingerprints must differ but matched:\n  A: {a}\n  B: {b}"
    );
}

#[test]
fn changed_projection_expression_differs() {
    let a = format!("{SEED} SELECT id, total FROM data");
    let b = format!("{SEED} SELECT id, total * 2 AS total FROM data");
    assert_distinct(&a, &b);
}

#[test]
fn changed_filter_predicate_differs() {
    let a = format!("{SEED} SELECT id, total FROM data WHERE total > 0");
    let b = format!("{SEED} SELECT id, total FROM data WHERE total > 1");
    assert_distinct(&a, &b);
}

#[test]
fn added_filter_differs() {
    let a = format!("{SEED} SELECT id, total FROM data");
    let b = format!("{SEED} SELECT id, total FROM data WHERE total > 0");
    assert_distinct(&a, &b);
}

#[test]
fn added_column_differs() {
    let a = format!("{SEED} SELECT id FROM data");
    let b = format!("{SEED} SELECT id, total FROM data");
    assert_distinct(&a, &b);
}

#[test]
fn removed_column_differs() {
    let a = format!("{SEED} SELECT id, total FROM data");
    let b = format!("{SEED} SELECT id FROM data");
    assert_distinct(&a, &b);
}

#[test]
fn renamed_output_column_differs() {
    // Renaming an output column changes the relation (matched by name downstream).
    let a = format!("{SEED} SELECT id, total FROM data");
    let b = format!("{SEED} SELECT id, total AS amount FROM data");
    assert_distinct(&a, &b);
}

#[test]
fn distinct_added_differs() {
    let a = format!("{SEED} SELECT id, total FROM data");
    let b = format!("{SEED} SELECT DISTINCT id, total FROM data");
    assert_distinct(&a, &b);
}

#[test]
fn changed_group_by_differs() {
    let a = format!("{SEED} SELECT id, sum(total) AS s FROM data GROUP BY id");
    let b = format!("{SEED} SELECT id, sum(total) AS s FROM data GROUP BY id, total");
    assert_distinct(&a, &b);
}

// ---- Soundness guards: a join must not be dropped by derived-table inlining ----

/// A two-row derived-table body, used as both sides of a join. `a` values
/// {1, 4} and `b` values {2, 0} are disjoint, so a join on `a = b` matches
/// nothing — a row-filtering change that must move the fingerprint.
const JOIN_BODY: &str = "SELECT 1 AS a, 2 AS b UNION ALL SELECT 4, 0";

#[test]
fn added_join_to_derived_table_differs() {
    // The left side is a derived table; the inliner must not represent the query
    // by the left subquery alone and silently drop the JOIN. The join filters
    // every row (0 rows vs 2), so the fingerprint must differ.
    let a = format!("SELECT l.a FROM ({JOIN_BODY}) AS l");
    let b =
        format!("SELECT l.a FROM ({JOIN_BODY}) AS l INNER JOIN ({JOIN_BODY}) AS r ON l.a = r.b");
    assert_distinct(&a, &b);
}

#[test]
fn inner_vs_left_join_on_derived_table_differs() {
    // Same shape, INNER vs LEFT: with disjoint keys, INNER yields 0 rows and
    // LEFT yields 2 (with NULLs). The join kind must be in the fingerprint.
    let a =
        format!("SELECT l.a FROM ({JOIN_BODY}) AS l INNER JOIN ({JOIN_BODY}) AS r ON l.a = r.b");
    let b = format!("SELECT l.a FROM ({JOIN_BODY}) AS l LEFT JOIN ({JOIN_BODY}) AS r ON l.a = r.b");
    assert_distinct(&a, &b);
}

#[test]
fn changed_join_condition_on_derived_table_differs() {
    // ON l.a = r.a (every row matches itself, 2 rows) vs ON l.a = r.b (0 rows).
    let a =
        format!("SELECT l.a FROM ({JOIN_BODY}) AS l INNER JOIN ({JOIN_BODY}) AS r ON l.a = r.a");
    let b =
        format!("SELECT l.a FROM ({JOIN_BODY}) AS l INNER JOIN ({JOIN_BODY}) AS r ON l.a = r.b");
    assert_distinct(&a, &b);
}

// ---- Soundness guards: row-affecting tail clauses must move the fingerprint ----

/// A three-row body so LIMIT/OFFSET have material effect.
const TAIL_BODY: &str = "SELECT 1 AS a UNION ALL SELECT 2 UNION ALL SELECT 3";

#[test]
fn added_limit_differs() {
    // LIMIT changes the row set (3 rows vs 1); it must be in the fingerprint.
    let a = format!("SELECT a FROM ({TAIL_BODY}) AS t");
    let b = format!("SELECT a FROM ({TAIL_BODY}) AS t LIMIT 1");
    assert_distinct(&a, &b);
}

#[test]
fn changed_limit_count_differs() {
    let a = format!("SELECT a FROM ({TAIL_BODY}) AS t LIMIT 1");
    let b = format!("SELECT a FROM ({TAIL_BODY}) AS t LIMIT 2");
    assert_distinct(&a, &b);
}

#[test]
fn added_offset_differs() {
    let a = format!("SELECT a FROM ({TAIL_BODY}) AS t LIMIT 1");
    let b = format!("SELECT a FROM ({TAIL_BODY}) AS t LIMIT 1 OFFSET 1");
    assert_distinct(&a, &b);
}

#[test]
fn order_by_direction_under_limit_differs() {
    // ORDER BY decides which row survives LIMIT 1; ASC vs DESC pick different rows.
    let a = format!("SELECT a FROM ({TAIL_BODY}) AS t ORDER BY a ASC LIMIT 1");
    let b = format!("SELECT a FROM ({TAIL_BODY}) AS t ORDER BY a DESC LIMIT 1");
    assert_distinct(&a, &b);
}

#[test]
fn qualify_filter_differs() {
    // QUALIFY filters rows by a window condition — a real row-set change.
    let a = format!("SELECT a FROM ({TAIL_BODY}) AS t");
    let b = format!("SELECT a FROM ({TAIL_BODY}) AS t QUALIFY row_number() OVER (ORDER BY a) = 1");
    assert_distinct(&a, &b);
}

// ---- Soundness guards: reorder must NOT collapse where position is observable ----

#[test]
fn union_branch_reorder_is_not_collapsed() {
    // UNION matches columns by position; reordering a branch's projection
    // changes the result, so the fingerprint must differ. The set-operation
    // verbatim fallback guarantees this.
    let a = "SELECT 1 AS a, 2 AS b UNION ALL SELECT 3 AS a, 4 AS b";
    let b = "SELECT 2 AS b, 1 AS a UNION ALL SELECT 3 AS a, 4 AS b";
    assert_distinct(a, b);
}
