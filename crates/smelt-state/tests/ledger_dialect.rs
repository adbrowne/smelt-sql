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
    let fold_record =
        ledger::ledger_fold_record_sql(dialect, "ds", "m", "{*}", "smelt.src", "d", "s", "e")
            .ok()?;
    let exists = ledger::ledger_exists_sql(dialect, "ds", "m", "{*}", "smelt.src", "d").ok()?;
    let reset =
        ledger::ledger_recompute_reset_sqls(dialect, "ds", "m", "{*}", "s", "e", "smelt.src", "d")
            .ok()?;
    let mut out = vec![table, insert, fold_record, upsert, exists];
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

/// GoogleSQL does not continue a string on `''` — it is two adjacent empty
/// strings — and a backslash IS an escape character there, so a quote-doubled
/// literal is a silent value corruption rather than a syntax error. Every
/// statement the BigQuery path can produce must use the backslash form.
#[test]
fn the_bigquery_path_never_doubles_a_quote_to_escape_one() {
    let table = ledger::ledger_table_ddl(SqlDialect::BigQuery, "ds").expect("BigQuery DDL");
    let quoted_model = "o'brien\\co";
    let mut statements = vec![table];
    statements.push(
        ledger::ledger_insert_sql(
            SqlDialect::BigQuery,
            "ds",
            quoted_model,
            "{*}",
            "smelt.src",
            "d",
            "s",
            "e",
        )
        .expect("BigQuery insert"),
    );
    statements.push(
        ledger::ledger_fold_record_sql(
            SqlDialect::BigQuery,
            "ds",
            quoted_model,
            "{*}",
            "smelt.src",
            "d",
            "s",
            "e",
        )
        .expect("BigQuery fold record"),
    );
    statements.push(
        ledger::ledger_exists_sql(
            SqlDialect::BigQuery,
            "ds",
            quoted_model,
            "{*}",
            "smelt.src",
            "d",
        )
        .expect("BigQuery exists"),
    );
    for sql in &statements {
        assert!(!sql.contains("''"), "quote-doubled escape: {sql}");
    }
}

/// The never-fold-twice record is the phase's whole point, and its two
/// realisations must be *different statements* that mean the same thing.
/// DuckDB's refusal is the enforced `PRIMARY KEY`, so a plain `INSERT`
/// suffices; BigQuery's key is `NOT ENFORCED`, so the record has to insert
/// zero rows on a repeat instead — a `MERGE … WHEN NOT MATCHED`, whose row
/// count the backend seam reads. A BigQuery fold record that is a plain
/// `INSERT` silently double-counts, which is exactly what this asserts
/// against.
#[test]
fn the_fold_record_refuses_a_repeat_in_each_dialects_own_way() {
    let duckdb =
        ledger::ledger_fold_record_sql(SqlDialect::DuckDB, "main", "m", "g", "i", "d", "s", "e")
            .expect("DuckDB fold record");
    assert!(
        duckdb.starts_with("INSERT INTO"),
        "DuckDB refuses by enforced PRIMARY KEY on a plain INSERT: {duckdb}"
    );

    let bigquery =
        ledger::ledger_fold_record_sql(SqlDialect::BigQuery, "ds", "m", "g", "i", "d", "s", "e")
            .expect("BigQuery fold record");
    assert!(
        bigquery.starts_with("MERGE "),
        "BigQuery's PRIMARY KEY is NOT ENFORCED, so a plain INSERT would double-count: {bigquery}"
    );
    assert!(
        bigquery.contains("WHEN NOT MATCHED THEN INSERT"),
        "the repeat must modify zero rows: {bigquery}"
    );
    assert!(
        !bigquery.contains("WHEN MATCHED"),
        "a matched arm would modify a row on a repeat and defeat the row-count test: {bigquery}"
    );

    assert!(
        ledger::ledger_fold_record_sql(SqlDialect::SparkSQL, "ds", "m", "g", "i", "d", "s", "e")
            .is_err(),
        "Spark has no sound never-fold-twice realisation"
    );
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
