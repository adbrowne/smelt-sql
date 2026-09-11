//! The ledger dispatch (`smelt_state::ledger`) must be exhaustive over every
//! dialect, and the dialect it dispatches to must never leak another
//! dialect's spelling.
//!
//! This is the ledger's sibling of
//! `cargo test -p smelt-logical --test maintenance_dialect_blindness`, which
//! guards the *maintenance emitters* in `smelt-logical`. The ledger builders
//! live in `smelt-state` under the bookkeeping exclusion from the
//! maintenance-plan-purity invariant (`CLAUDE.md` §"Maintenance-plan
//! purity"), so they need their own gate here rather than being covered by
//! that one.

use smelt_dialect::SqlDialect;
use smelt_state::ledger;

/// Every `SqlDialect`. Kept exhaustive by [`every_dialect_is_covered`].
const ALL_DIALECTS: [SqlDialect; 3] = [
    SqlDialect::DuckDB,
    SqlDialect::SparkSQL,
    SqlDialect::BigQuery,
];

/// Every statement the dispatch can build, for one dialect. `None` where the
/// dialect has no ledger spelling at all.
fn all_statements(dialect: SqlDialect) -> Option<Vec<String>> {
    let table = ledger::ledger_table_ddl(dialect, "ds").ok()?;
    let insert =
        ledger::ledger_insert_sql(dialect, "ds", "m", "{*}", "smelt.src", "d", "s", "e").ok()?;
    let upsert =
        ledger::ledger_upsert_sql(dialect, "ds", "m", "{*}", "smelt.src", "d", "s", "e").ok()?;
    let exists = ledger::ledger_exists_sql(dialect, "ds", "m", "{*}", "smelt.src", "d").ok()?;
    let reset =
        ledger::ledger_recompute_reset_sqls(dialect, "ds", "m", "{*}", "s", "e", "smelt.src", "d")
            .ok()?;
    let mut out = vec![table, insert, upsert, exists];
    out.extend(reset);
    Some(out)
}

#[test]
fn every_dialect_is_covered() {
    for dialect in ALL_DIALECTS {
        // A new `SqlDialect` variant makes this `match` a compile error, which
        // is the point — the array above must then grow too, and so must the
        // dispatch's own exhaustive `match`.
        match dialect {
            SqlDialect::DuckDB | SqlDialect::SparkSQL | SqlDialect::BigQuery => {}
        }
        // Every dialect answers — with text or with a refusal, never a panic.
        let _ = all_statements(dialect);
    }
}

/// Spark has no sound realisation (`docs/specs/state.md` §"Which dialects
/// realise which structure"), so asking for its ledger text is an error
/// naming the dialect — never DuckDB SQL Spark cannot run.
#[test]
fn spark_is_refused_by_name_rather_than_handed_duckdb_sql() {
    let err = ledger::ledger_table_ddl(SqlDialect::SparkSQL, "ds")
        .expect_err("Spark must not receive ledger SQL");
    assert_eq!(err.dialect, SqlDialect::SparkSQL.name());
    assert!(
        err.to_string().contains(SqlDialect::SparkSQL.name()),
        "the refusal must name the dialect: {err}"
    );
    assert!(all_statements(SqlDialect::SparkSQL).is_none());
}

/// The BigQuery path must carry no DuckDB-flavoured text: no double-quoted
/// identifier (a *string literal* in GoogleSQL), no `ON CONFLICT`, no
/// `VARCHAR`, and no quote-doubled string escape.
#[test]
fn the_bigquery_path_is_never_duckdb_flavoured() {
    let statements =
        all_statements(SqlDialect::BigQuery).expect("BigQuery realises the merge ledger");
    for sql in &statements {
        assert!(!sql.contains('"'), "double-quoted identifier: {sql}");
        assert!(!sql.contains("ON CONFLICT"), "ON CONFLICT: {sql}");
        assert!(!sql.contains("VARCHAR"), "VARCHAR: {sql}");
        assert!(
            sql.contains("`ds._smelt_ledger`"),
            "unbackticked name: {sql}"
        );
    }
    let upsert =
        ledger::ledger_upsert_sql(SqlDialect::BigQuery, "ds", "m", "g", "i", "d", "s", "e")
            .expect("BigQuery upsert");
    assert!(upsert.starts_with("MERGE "), "{upsert}");
    assert!(upsert.contains("WHEN NOT MATCHED THEN INSERT"), "{upsert}");
}

/// …and the DuckDB path is unchanged by the dispatch — byte-identical to the
/// builders it delegates to, so routing the driver through this module moved
/// no DuckDB statement.
#[test]
fn the_duckdb_path_is_byte_identical_to_the_duckdb_builders() {
    use smelt_state::ddl_duckdb;
    assert_eq!(
        ledger::ledger_table_ddl(SqlDialect::DuckDB, "main").unwrap(),
        ddl_duckdb::generate_ledger_table_ddl("main")
    );
    assert_eq!(
        ledger::ledger_upsert_sql(SqlDialect::DuckDB, "main", "m", "g", "i", "d", "s", "e")
            .unwrap(),
        ddl_duckdb::generate_ledger_upsert_sql("main", "m", "g", "i", "d", "s", "e")
    );
    assert_eq!(
        ledger::ledger_insert_sql(SqlDialect::DuckDB, "main", "m", "g", "i", "d", "s", "e")
            .unwrap(),
        ddl_duckdb::generate_ledger_insert_sql("main", "m", "g", "i", "d", "s", "e")
    );
    assert_eq!(
        ledger::ledger_exists_sql(SqlDialect::DuckDB, "main", "m", "g", "i", "d").unwrap(),
        ddl_duckdb::generate_ledger_exists_sql("main", "m", "g", "i", "d")
    );
    assert_eq!(
        ledger::ledger_recompute_reset_sqls(
            SqlDialect::DuckDB,
            "main",
            "m",
            "g",
            "s",
            "e",
            "i",
            "d"
        )
        .unwrap(),
        ddl_duckdb::generate_ledger_recompute_reset_sqls("main", "m", "g", "s", "e", "i", "d")
    );
}

/// Non-vacuity: the two dialects that DO have a spelling must not produce the
/// same text, or the leak assertions above would pass on an accidental
/// single-dialect dispatch.
#[test]
fn the_two_realising_dialects_produce_different_text() {
    let duckdb = all_statements(SqlDialect::DuckDB).expect("DuckDB realises the merge ledger");
    let bigquery =
        all_statements(SqlDialect::BigQuery).expect("BigQuery realises the merge ledger");
    assert_eq!(duckdb.len(), bigquery.len());
    for (d, b) in duckdb.iter().zip(bigquery.iter()) {
        assert_ne!(d, b, "the dispatch returned DuckDB text for BigQuery");
    }
}
