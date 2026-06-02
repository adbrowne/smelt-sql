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
