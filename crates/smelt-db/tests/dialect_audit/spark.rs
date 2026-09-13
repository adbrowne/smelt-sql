//! Spark. Skips green when `SPARK_CONTAINER_ID` is unset; a live Spark Connect
//! container is a labelled-PR / nightly cost, not a per-PR one.

use crate::legs::{run_schema_leg, run_value_leg};
use crate::probe;
use smelt_oracle_testkit::{compare_cells, DuckDbOracle, SparkOracle, ValueMatch, ValueOracle};
use smelt_types::DialectId;
use std::sync::LazyLock;

static SPARK: LazyLock<Option<SparkOracle>> = LazyLock::new(|| {
    std::env::var("SPARK_CONTAINER_ID")
        .ok()
        .filter(|id| !id.is_empty())
        .map(|id| SparkOracle::new(&id))
});

#[test]
fn schema_leg_spark() {
    let Some(oracle) = SPARK.as_ref() else {
        eprintln!("SPARK_CONTAINER_ID unset — skipping schema_leg_spark");
        return;
    };
    let outcome = run_schema_leg(DialectId::SparkSql, oracle);
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    eprintln!(
        "COVERAGE[spark schema] probes_compared={}",
        outcome.probes_compared
    );
}

#[test]
fn value_leg_spark() {
    let Some(oracle) = SPARK.as_ref() else {
        eprintln!("SPARK_CONTAINER_ID unset — skipping value_leg_spark");
        return;
    };
    let outcome = run_value_leg(DialectId::SparkSql, oracle, &DuckDbOracle::new());
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    eprintln!(
        "COVERAGE[spark value] probes_compared={}",
        outcome.probes_compared
    );
}

/// The regression test for the finding that motivated this work: Spark's infix
/// `^` is bitwise XOR, not exponentiation. Before the emission row that lowers
/// it to `POWER(a, b)`, `SELECT 2 ^ 3` returned 1 on Spark and 8 on DuckDB — a
/// silently wrong number, not an error, and the same type on both engines, so
/// no schema comparison could see it.
#[test]
fn spark_caret_agrees_with_duckdb_power() {
    let Some(spark) = SPARK.as_ref() else {
        eprintln!("SPARK_CONTAINER_ID unset — skipping spark_caret_agrees_with_duckdb_power");
        return;
    };
    let duckdb = DuckDbOracle::new();
    let smelt_expr = "SELECT n_bigint ^ 2 AS p FROM fixture ORDER BY rid";
    let spark_rows = spark
        .execute_rows(&probe::print_for(DialectId::SparkSql, smelt_expr))
        .expect("spark");
    let duck_rows = duckdb
        .execute_rows(&probe::print_for(DialectId::DuckDb, smelt_expr))
        .expect("duckdb");
    assert_eq!(spark_rows.len(), duck_rows.len());
    for (s, d) in spark_rows.iter().zip(&duck_rows) {
        assert_eq!(
            compare_cells(&d[0], &s[0]),
            ValueMatch::Equal,
            "`^` diverges on Spark: it is bitwise XOR there, and must be lowered to POWER"
        );
    }
}
