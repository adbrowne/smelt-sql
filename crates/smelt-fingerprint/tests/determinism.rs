//! Determinism detector — a model whose output is not a pure function of its
//! inputs cannot have its output *proven* equal across versions, so a
//! fingerprint match must not be treated as relation-equality for it (see
//! `docs/research/20260601-virtual-environments.md` §5.5).
//!
//! The detector is structural: it flags a small deny-list of non-deterministic
//! built-ins (inline, with no function property to read) plus row-slicing tail
//! clauses (`LIMIT`/`OFFSET`/`FETCH`) that pick *which* rows survive without a
//! provably total order. It is conservative — flagging more than strictly
//! necessary is sound (worst case: the model is rebuilt rather than reused); the
//! load-bearing invariant is the converse, that anything flagged `deterministic`
//! really is reproducible.

use smelt_fingerprint::output_fingerprint_from_sql;

#[track_caller]
fn deterministic(sql: &str) -> bool {
    output_fingerprint_from_sql(sql, &[])
        .expect("parses to a SELECT")
        .deterministic
}

// ---- Deterministic models: pure functions of their inputs ----

#[test]
fn plain_projection_is_deterministic() {
    assert!(deterministic(
        "SELECT a, b FROM (SELECT 1 AS a, 2 AS b) AS t"
    ));
}

#[test]
fn arithmetic_and_pure_functions_are_deterministic() {
    assert!(deterministic(
        "SELECT abs(a) AS x, a + b AS y FROM (SELECT 1 AS a, 2 AS b) AS t"
    ));
}

#[test]
fn order_insensitive_aggregates_are_deterministic() {
    // sum/count/min/max/avg do not depend on input row order, so they are pure
    // functions of the input multiset and must not be flagged.
    let body = "(SELECT 1 AS a UNION ALL SELECT 2 UNION ALL SELECT 3) AS t";
    for agg in ["sum(a)", "count(a)", "min(a)", "max(a)", "avg(a)"] {
        assert!(
            deterministic(&format!("SELECT {agg} AS s FROM {body}")),
            "{agg} should be deterministic"
        );
    }
}

#[test]
fn bare_order_by_without_limit_is_deterministic() {
    // An ORDER BY with no row-slicing clause only reorders an order-insensitive
    // (multiset-by-name) relation, so it does not introduce non-determinism.
    assert!(deterministic(
        "SELECT a FROM (SELECT 1 AS a UNION ALL SELECT 2) AS t ORDER BY a"
    ));
}

// ---- Non-deterministic built-ins (inline, no function property to read) ----

#[test]
fn random_is_non_deterministic() {
    assert!(!deterministic(
        "SELECT random() AS r, a FROM (SELECT 1 AS a) AS t"
    ));
}

#[test]
fn now_is_non_deterministic() {
    assert!(!deterministic(
        "SELECT now() AS ts, a FROM (SELECT 1 AS a) AS t"
    ));
}

#[test]
fn uuid_is_non_deterministic() {
    assert!(!deterministic(
        "SELECT uuid() AS u, a FROM (SELECT 1 AS a) AS t"
    ));
}

#[test]
fn bare_current_timestamp_is_non_deterministic() {
    // DuckDB accepts the temporal specials without parentheses; there is no
    // FUNCTION_CALL node, so the detector must catch the bare identifier form.
    assert!(!deterministic(
        "SELECT current_timestamp AS ts, a FROM (SELECT 1 AS a) AS t"
    ));
}

// ---- Row-slicing tail clauses without a provably total order ----

#[test]
fn limit_is_non_deterministic() {
    assert!(!deterministic(
        "SELECT a FROM (SELECT 1 AS a UNION ALL SELECT 2 UNION ALL SELECT 3) AS t LIMIT 1"
    ));
}

#[test]
fn limit_offset_is_non_deterministic() {
    assert!(!deterministic(
        "SELECT a FROM (SELECT 1 AS a UNION ALL SELECT 2 UNION ALL SELECT 3) AS t LIMIT 1 OFFSET 1"
    ));
}

#[test]
fn order_by_under_limit_is_non_deterministic_conservatively() {
    // Even with an ORDER BY, the detector cannot cheaply prove the sort key is a
    // total order (ties make `ORDER BY x LIMIT n` non-deterministic), so it
    // conservatively flags every row-slice. Refining this to "ORDER BY a
    // provably-unique key ⇒ deterministic" needs key-uniqueness information the
    // fingerprinter does not have yet.
    assert!(!deterministic(
        "SELECT a FROM (SELECT 1 AS a UNION ALL SELECT 2) AS t ORDER BY a LIMIT 1"
    ));
}

// ---- Order-sensitive aggregates without a (total) inner ORDER BY ----

#[test]
fn order_sensitive_aggregates_are_non_deterministic() {
    // These aggregates' results depend on input row order, which a relation (an
    // unordered multiset) does not fix. smelt has no aggregate-`ORDER BY` syntax
    // to pin the order, so every occurrence is non-deterministic. (`first`/`last`
    // are order-sensitive too but are keywords, so they cannot be written as
    // aggregate calls today — see the detector's deny-list note.)
    let body = "(SELECT 1 AS a UNION ALL SELECT 2 UNION ALL SELECT 3) AS t";
    for agg in [
        "array_agg(a)",
        "list(a)",
        "string_agg(a, ',')",
        "group_concat(a)",
        "listagg(a, ',')",
        "any_value(a)",
        "arbitrary(a)",
    ] {
        assert!(
            !deterministic(&format!("SELECT {agg} AS s FROM {body}")),
            "{agg} should be non-deterministic"
        );
    }
}

// ---- Recursion: non-determinism anywhere in the expansion taints the model ----

#[test]
fn non_determinism_in_subquery_is_detected() {
    // The non-deterministic call is inside a derived table; the model's output
    // still depends on it, so the whole model is non-deterministic.
    assert!(!deterministic(
        "SELECT s.r FROM (SELECT random() AS r FROM (SELECT 1 AS a) AS i) AS s"
    ));
}

#[test]
fn non_determinism_in_cte_is_detected() {
    assert!(!deterministic(
        "WITH c AS (SELECT random() AS r) SELECT c.r FROM c"
    ));
}

// ---- The flag is independent of canonicalisability and of the fingerprint ----

#[test]
fn non_deterministic_model_can_still_be_canonicalisable() {
    // `SELECT random() AS r FROM t` is fully structured (no verbatim fallback),
    // yet non-deterministic: the two signals are orthogonal.
    let r = output_fingerprint_from_sql("SELECT random() AS r, a FROM (SELECT 1 AS a) AS t", &[])
        .expect("parses");
    assert!(r.canonicalisable, "expected a structured canonical form");
    assert!(!r.deterministic, "expected non-deterministic");
}

#[test]
fn identical_non_deterministic_sql_still_shares_a_fingerprint() {
    // Determinism is metadata for the reuse decision, not a fingerprint input:
    // the same query text fingerprints identically whether or not it is
    // deterministic. (The reuse layer is what must consult `deterministic`.)
    let a = output_fingerprint_from_sql("SELECT random() AS r FROM (SELECT 1 AS a) AS t", &[])
        .expect("parses");
    let b = output_fingerprint_from_sql("SELECT random() AS r FROM (SELECT 1 AS a) AS t", &[])
        .expect("parses");
    assert_eq!(a.fingerprint, b.fingerprint);
    assert!(!a.deterministic && !b.deterministic);
}
