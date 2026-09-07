//! `DATE_ADD`/`DATE_SUB` template rows, exercised through the real `print`
//! entry point, registry row and all, rather than the synthetic
//! `print_template_str` harness in [`super::pinned_output`].

use super::{duckdb, spark};

/// `DATE_SUB` is the first *function-call* template row (phase 4 of
/// `docs/outcomes/20260904-dialect-emission-vocabulary`) — DuckDB spells
/// interval subtraction infix.
#[test]
fn date_sub_lowers_to_infix_subtraction_on_duckdb() {
    // `{0} - {1}` is not call-shaped, so its whole output is wrapped in
    // parens (`non_call_template_is_wrapped_in_parens`'s rule) so it composes
    // safely wherever the original call appeared.
    assert_eq!(
        duckdb("SELECT DATE_SUB(d, INTERVAL 1 DAY) FROM t"),
        "SELECT (d - INTERVAL 1 DAY) FROM t"
    );
    // A compound first argument picks up its own inner parenthesisation too.
    assert_eq!(
        duckdb("SELECT DATE_SUB(a + b, INTERVAL 1 DAY) FROM t"),
        "SELECT ((a + b) - INTERVAL 1 DAY) FROM t"
    );
}

/// Spark reports the bare infix `DATE + INTERVAL` as `DATE`, not `TIMESTAMP`
/// — the declared smelt (and DuckDB) return type — so the Spark template
/// wraps the infix form in an explicit cast. Verified live 2026-09-06
/// (phase 9 of docs/outcomes/20260904-dialect-emission-vocabulary).
#[test]
fn date_add_prints_the_spark_form() {
    assert_eq!(
        spark("SELECT DATE_ADD(d, INTERVAL 1 DAY) FROM t"),
        "SELECT CAST(d + INTERVAL 1 DAY AS TIMESTAMP) FROM t"
    );
    // DuckDB is unaffected — `DATE_ADD` keeps its `Native` verdict there.
    assert_eq!(
        duckdb("SELECT DATE_ADD(d, INTERVAL 1 DAY) FROM t"),
        "SELECT DATE_ADD(d, INTERVAL 1 DAY) FROM t"
    );
}

/// Same reasoning as `date_add_prints_the_spark_form`, for `DATE_SUB` — the
/// cast is the spelling whose Spark-reported type matches smelt's declared
/// `Timestamp` return type, not merely a spelling that parses.
#[test]
fn date_sub_spark_form_matches_smelt_return_type() {
    assert_eq!(
        spark("SELECT DATE_SUB(d, INTERVAL 1 DAY) FROM t"),
        "SELECT CAST(d - INTERVAL 1 DAY AS TIMESTAMP) FROM t"
    );
}
