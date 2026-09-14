//! Trino. Both legs live. Skips green when `SMELT_TRINO_URL` is unset; the
//! tier is `scripts/trino-up.sh` + `scripts/trino-env.sh`, a per-PR/nightly
//! cost, never a bare-Docker ambient assumption.

use crate::legs::{run_schema_leg, run_value_leg};
use crate::probe;
use smelt_oracle_testkit::{compare_cells, DuckDbOracle, TrinoOracle, ValueMatch, ValueOracle};
use smelt_types::DialectId;
use std::sync::LazyLock;

static TRINO: LazyLock<Option<TrinoOracle>> = LazyLock::new(TrinoOracle::from_env);

#[test]
fn schema_leg_trino() {
    let Some(oracle) = TRINO.as_ref() else {
        eprintln!("SMELT_TRINO_URL unset — skipping schema_leg_trino");
        return;
    };
    let outcome = run_schema_leg(DialectId::Trino, oracle);
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    assert!(
        outcome.probes_compared >= crate::legs::PROBE_COVERAGE_FLOOR,
        "too few probes compared: {}",
        outcome.probes_compared
    );
    eprintln!(
        "COVERAGE[trino schema] probes_compared={}",
        outcome.probes_compared
    );
}

#[test]
fn value_leg_trino() {
    let Some(oracle) = TRINO.as_ref() else {
        eprintln!("SMELT_TRINO_URL unset — skipping value_leg_trino");
        return;
    };
    let outcome = run_value_leg(DialectId::Trino, oracle, &DuckDbOracle::new());
    assert!(outcome.failures.is_empty(), "{}", outcome.report());
    assert!(
        outcome.probes_compared >= crate::legs::PROBE_COVERAGE_FLOOR,
        "too few probes compared: {}",
        outcome.probes_compared
    );
    eprintln!(
        "COVERAGE[trino value] probes_compared={}",
        outcome.probes_compared
    );
}

/// The regression analogue of `spark_caret_agrees_with_duckdb_power` and
/// `bigquery_caret_agrees_with_duckdb_power`: this leg catches a spelling
/// that survives but changes meaning, which a schema comparison cannot see.
/// Trino has no infix `^` operator at all, so the registry lowers it to
/// `POWER({0}, {1})` (phase 3) — this proves that template computes the same
/// number DuckDB's native `^` does.
#[test]
fn trino_caret_agrees_with_duckdb_power() {
    let Some(trino) = TRINO.as_ref() else {
        eprintln!("SMELT_TRINO_URL unset — skipping trino_caret_agrees_with_duckdb_power");
        return;
    };
    let duckdb = DuckDbOracle::new();
    let smelt_expr = "SELECT n_bigint ^ 2 AS p FROM fixture ORDER BY rid";
    let trino_rows = trino
        .execute_rows(&probe::print_for(DialectId::Trino, smelt_expr))
        .expect("trino");
    let duck_rows = duckdb
        .execute_rows(&probe::print_for(DialectId::DuckDb, smelt_expr))
        .expect("duckdb");
    assert_eq!(trino_rows.len(), duck_rows.len());
    for (t, d) in trino_rows.iter().zip(&duck_rows) {
        assert_eq!(
            compare_cells(&d[0], &t[0]),
            ValueMatch::Equal,
            "`^` diverges on Trino: it has no infix `^` operator and must lower to POWER"
        );
    }
}
