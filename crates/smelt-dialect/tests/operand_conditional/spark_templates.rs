//! Printed-output assertions for the Spark `Emission::Template` rows.

use smelt_dialect::{BackendCapabilities, SqlDialect};

use super::print_with;

#[test]
fn dayofweek_prints_the_shift_template_on_spark() {
    let sql = "SELECT DAYOFWEEK(d) FROM t";
    assert_eq!(
        print_with(sql, &SqlDialect::SparkSQL, &BackendCapabilities::spark()),
        "SELECT (DAYOFWEEK(d) - 1) FROM t",
        "the non-call template's whole output must be parenthesised"
    );
    assert_eq!(
        print_with(sql, &SqlDialect::DuckDB, &BackendCapabilities::duckdb()),
        "SELECT DAYOFWEEK(d) FROM t"
    );
}

/// Phase 8's Spark `Emission::Template` rows — `AGE`, `DATE_SUB`,
/// `TO_SECONDS` — closing #178. Each is call-shaped, so its own arguments
/// substitute bare (no extra wrapping); `AGE`/`DATE_SUB` land on a
/// `BINARY_EXPR`-equivalent shape and their whole non-call output is
/// parenthesised, matching `DAYOFWEEK`'s precedent above.
#[test]
fn age_prints_the_spark_form() {
    assert_eq!(
        print_with(
            "SELECT AGE(a, b) FROM t",
            &SqlDialect::SparkSQL,
            &BackendCapabilities::spark()
        ),
        "SELECT (a - b) FROM t"
    );
}

#[test]
fn date_sub_prints_the_spark_form() {
    // Phase 9: the bare infix form reports `DATE` on Spark, not smelt's
    // declared `Timestamp` return type, so the template carries an explicit
    // cast (docs/outcomes/20260904-dialect-emission-vocabulary phase 9).
    assert_eq!(
        print_with(
            "SELECT DATE_SUB(a, b) FROM t",
            &SqlDialect::SparkSQL,
            &BackendCapabilities::spark()
        ),
        "SELECT CAST(a - b AS TIMESTAMP) FROM t"
    );
}

#[test]
fn to_seconds_prints_the_spark_form() {
    assert_eq!(
        print_with(
            "SELECT TO_SECONDS(a) FROM t",
            &SqlDialect::SparkSQL,
            &BackendCapabilities::spark()
        ),
        "SELECT make_interval(0, 0, 0, 0, 0, 0, a) FROM t"
    );
}
