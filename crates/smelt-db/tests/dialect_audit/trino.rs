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

/// Live regression for `docs/outcomes/20260913-trino-emission` phase 7:
/// `ARG_MAX` (smelt's name, Trino spells it `MAX_BY`) as a whole-partition
/// window function, over a fixture whose grouping column includes a NULL —
/// the value the restructure path's null-safe join exists to protect
/// elsewhere, exercised here on Trino's *native* window form instead (no
/// restructure/join is synthesised, since Trino accepts `MAX_BY` as a window
/// function in every position — measured live 2026-09-14). Compiles smelt's
/// `ARG_MAX` through the real registry lowering for both engines and asserts
/// row-for-row agreement, including the NULL-group row.
#[test]
fn trino_arg_max_window_agrees_with_duckdb_native() {
    let Some(trino) = TRINO.as_ref() else {
        eprintln!(
            "SMELT_TRINO_URL unset — skipping trino_arg_max_window_agrees_with_duckdb_native"
        );
        return;
    };
    let duckdb = DuckDbOracle::new();
    let smelt_sql = "WITH t AS (SELECT * FROM (VALUES \
                     ('a', 1.0, 1), ('a', 3.0, 2), ('a', 2.0, 3), \
                     ('b', 5.0, 1), ('b', 4.0, 2), \
                     (CAST(NULL AS VARCHAR), 9.0, 1)) AS v(g, x, t)) \
                     SELECT g, t, ARG_MAX(x, t) OVER (PARTITION BY g) AS best \
                     FROM t ORDER BY t, g";

    let trino_rows = trino
        .execute_rows(&probe::print_for(DialectId::Trino, smelt_sql))
        .expect("trino");
    let duck_rows = duckdb
        .execute_rows(&probe::print_for(DialectId::DuckDb, smelt_sql))
        .expect("duckdb");
    assert_eq!(trino_rows.len(), duck_rows.len());
    assert!(!trino_rows.is_empty(), "expected non-empty fixture rows");
    for (t, d) in trino_rows.iter().zip(&duck_rows) {
        for (col, (tc, dc)) in t.iter().zip(d).enumerate() {
            assert_eq!(
                compare_cells(dc, tc),
                ValueMatch::Equal,
                "column {col} diverges between Trino and DuckDB for ARG_MAX/MAX_BY \
                 over a NULL-group window: trino={tc:?} duckdb={dc:?}"
            );
        }
    }
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
