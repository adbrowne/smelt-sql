//! The promoted crate is usable from outside `smelt-db`'s test tree.

use smelt_oracle_testkit::{classify_oracle_error, DuckDbOracle, OracleErrorKind, TypeOracle};
use smelt_types::DataType;

#[test]
fn the_duckdb_oracle_reports_a_schema() {
    let oracle = DuckDbOracle::new();
    let cols = oracle
        .query_types("SELECT 1 AS a, 'x' AS b")
        .expect("query");
    assert_eq!(cols.len(), 2);
    assert_eq!(cols[0].0, "a");
}

#[test]
fn a_binder_error_is_a_refusal_not_a_harness_failure() {
    assert_eq!(
        classify_oracle_error("Binder Error: no such column"),
        OracleErrorKind::QueryRefusal
    );
    assert_eq!(
        classify_oracle_error("connection reset"),
        OracleErrorKind::Fatal
    );
}

#[test]
fn compare_types_is_reachable_from_the_testkit() {
    use smelt_oracle_testkit::{compare_types, TypeMatch};
    assert_eq!(
        compare_types(&DataType::BigInt, &DataType::BigInt),
        TypeMatch::Exact
    );
}
