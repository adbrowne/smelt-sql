//! BigQuery. Manual sweep only, per `multi_backend.md` §"BigQuery has no CI
//! tier, by decision, not by omission" — and more so here than anywhere else,
//! because the value leg *executes* rather than dry-runs, so it bills.
//!
//! Every test below skips green without a credential, which is why
//! `scripts/bigquery-dialect-audit.sh` refuses to start without one rather
//! than letting a green skip read as a passing sweep.

use crate::legs::{run_schema_leg, run_value_leg};
use crate::probe;
use smelt_oracle_testkit::{compare_cells, BigQueryOracle, DuckDbOracle, ValueMatch, ValueOracle};
use smelt_types::DialectId;
use std::sync::LazyLock;

static BIGQUERY: LazyLock<Option<BigQueryOracle>> = LazyLock::new(BigQueryOracle::from_env);

#[test]
fn schema_leg_bigquery() {
    let Some(oracle) = BIGQUERY.as_ref() else {
        eprintln!("BigQuery not configured — skipping schema_leg_bigquery");
        return;
    };
    let outcome = run_schema_leg(DialectId::BigQuery, oracle);
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    eprintln!(
        "COVERAGE[bigquery schema] probes_compared={}",
        outcome.probes_compared
    );
}

#[test]
fn value_leg_bigquery() {
    let Some(oracle) = BIGQUERY.as_ref() else {
        eprintln!("BigQuery not configured — skipping value_leg_bigquery");
        return;
    };
    let outcome = run_value_leg(DialectId::BigQuery, oracle, &DuckDbOracle::new());
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    eprintln!(
        "COVERAGE[bigquery value] probes_compared={}",
        outcome.probes_compared
    );
}

/// GoogleSQL defines infix `^` as bitwise XOR, exactly as Spark does. The same
/// silent-wrong-number hazard, on the backend where a wrong number is most
/// expensive to discover.
#[test]
fn bigquery_caret_agrees_with_duckdb_power() {
    let Some(bq) = BIGQUERY.as_ref() else {
        eprintln!("BigQuery not configured — skipping bigquery_caret_agrees_with_duckdb_power");
        return;
    };
    let duckdb = DuckDbOracle::new();
    let smelt_expr = "SELECT n_bigint ^ 2 AS p FROM fixture ORDER BY rid";
    let bq_rows = bq
        .execute_rows(&probe::print_for(DialectId::BigQuery, smelt_expr))
        .expect("bigquery");
    let duck_rows = duckdb
        .execute_rows(&probe::print_for(DialectId::DuckDb, smelt_expr))
        .expect("duckdb");
    assert_eq!(bq_rows.len(), duck_rows.len());
    for (b, d) in bq_rows.iter().zip(&duck_rows) {
        assert_eq!(
            compare_cells(&d[0], &b[0]),
            ValueMatch::Equal,
            "`^` diverges on BigQuery: GoogleSQL reads it as bitwise XOR"
        );
    }
}
