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
use smelt_state::{ledger, tombstone};
use smelt_types::DataType;

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

// ── observed-output-delta dispatch (`smelt_state::observed_delta`) ────────
//
// The same shape, and the same three obligations: exhaustive over the
// dialects, no other dialect's spelling leaking through, and Spark refused by
// name rather than handed SQL it cannot run.

const CHANGED_KEYS_QUERY: &str = "SELECT user_id AS delta_key, NULL AS delta_partition FROM main.t";

fn all_observed_delta_statements(dialect: SqlDialect) -> Option<Vec<String>> {
    use smelt_state::observed_delta as od;
    let table = od::observed_delta_table_ddl(dialect, "ds").ok()?;
    let upsert =
        od::observed_delta_upsert_sql(dialect, "ds", "m", "s", "e", CHANGED_KEYS_QUERY).ok()?;
    let select = od::observed_delta_select_sql(dialect, "ds", "m", "s", "e").ok()?;
    Some(vec![table, upsert, select])
}

/// Spark is refused by name, not given DuckDB SQL; every other dialect
/// resolves. A new dialect makes the `match` inside the dispatch a compile
/// error, and this test the place its verdict is stated.
#[test]
fn the_observed_delta_dispatch_is_exhaustive_and_refuses_spark() {
    use smelt_state::observed_delta as od;
    for dialect in ALL_DIALECTS {
        match dialect {
            SqlDialect::DuckDB | SqlDialect::BigQuery => {
                assert!(
                    all_observed_delta_statements(dialect).is_some(),
                    "{dialect:?} realises the observed-delta record"
                );
            }
            SqlDialect::SparkSQL => {
                let err = od::observed_delta_table_ddl(dialect, "ds")
                    .expect_err("Spark has no observed-delta spelling");
                assert_eq!(err.dialect, SqlDialect::SparkSQL.name());
                assert!(
                    err.to_string().contains(SqlDialect::SparkSQL.name()),
                    "the refusal must name the dialect: {err}"
                );
                assert!(od::observed_delta_upsert_sql(
                    dialect,
                    "ds",
                    "m",
                    "s",
                    "e",
                    CHANGED_KEYS_QUERY
                )
                .is_err());
                assert!(od::observed_delta_select_sql(dialect, "ds", "m", "s", "e").is_err());
            }
        }
    }
}

/// No DuckDB-ism may reach BigQuery through the observed-delta dispatch — the
/// same leak check the ledger half makes, over the constructs that actually
/// differ here.
#[test]
fn the_bigquery_observed_delta_path_carries_no_duckdb_spelling() {
    let statements =
        all_observed_delta_statements(SqlDialect::BigQuery).expect("BigQuery realises it");
    for sql in &statements {
        assert!(!sql.contains('"'), "double-quoted identifier: {sql}");
        assert!(!sql.contains("ON CONFLICT"), "{sql}");
        assert!(!sql.contains("FILTER (WHERE"), "{sql}");
        assert!(!sql.contains("VARCHAR"), "{sql}");
        assert!(!sql.contains("excluded."), "{sql}");
        assert!(
            !sql.contains("''"),
            "quote-doubling is not GoogleSQL escaping: {sql}"
        );
    }
}

/// The DuckDB observed-delta path is byte-identical to the builders it
/// delegates to — the dispatch adds no text of its own.
#[test]
fn the_duckdb_observed_delta_path_delegates_verbatim() {
    use smelt_state::{ddl_duckdb, observed_delta as od};
    assert_eq!(
        od::observed_delta_table_ddl(SqlDialect::DuckDB, "main").unwrap(),
        ddl_duckdb::generate_observed_delta_table_ddl("main")
    );
    assert_eq!(
        od::observed_delta_upsert_sql(
            SqlDialect::DuckDB,
            "main",
            "m",
            "s",
            "e",
            CHANGED_KEYS_QUERY
        )
        .unwrap(),
        ddl_duckdb::generate_observed_delta_upsert_sql("main", "m", "s", "e", CHANGED_KEYS_QUERY)
    );
    assert_eq!(
        od::observed_delta_select_sql(SqlDialect::DuckDB, "main", "m", "s", "e").unwrap(),
        ddl_duckdb::generate_observed_delta_select_sql("main", "m", "s", "e")
    );
}

/// Non-vacuity for the leak check above.
#[test]
fn the_two_realising_dialects_produce_different_observed_delta_text() {
    let duckdb = all_observed_delta_statements(SqlDialect::DuckDB).expect("DuckDB");
    let bigquery = all_observed_delta_statements(SqlDialect::BigQuery).expect("BigQuery");
    assert_eq!(duckdb.len(), bigquery.len());
    for (d, b) in duckdb.iter().zip(bigquery.iter()) {
        assert_ne!(d, b, "the dispatch returned DuckDB text for BigQuery");
    }
}

/// **Empty and absent are distinct, and on BigQuery the distinction is row
/// presence** (`docs/specs/incremental_models.md` §"The graph layer").
///
/// BigQuery cannot tell a NULL `ARRAY` from an empty one — a NULL written to
/// an `ARRAY` column reads back empty — so a realisation that encoded
/// "never recorded" as a NULL column would lose the distinction the moment it
/// crossed the wire. It does not: the upsert's source is one un-grouped
/// aggregate `SELECT`, so it writes exactly one row for the window even when
/// nothing changed (`COALESCE` folding the `NULL` aggregate to an empty
/// array), and the read filters on the window key alone. Absence is therefore
/// zero rows, which array flattening cannot manufacture or destroy.
#[test]
fn on_bigquery_empty_and_absent_are_separated_by_row_presence() {
    use smelt_state::observed_delta as od;
    let upsert = od::observed_delta_upsert_sql(
        SqlDialect::BigQuery,
        "ds",
        "m",
        "s",
        "e",
        CHANGED_KEYS_QUERY,
    )
    .unwrap();
    // Exactly one row is always written: an un-grouped aggregate source, and
    // both aggregates coalesced to the empty typed array.
    assert!(!upsert.contains("GROUP BY"), "{upsert}");
    assert_eq!(upsert.matches("ARRAY<STRING>[])").count(), 2, "{upsert}");
    // …and it is an upsert, so a re-run replaces rather than duplicating.
    assert!(upsert.contains("WHEN MATCHED THEN UPDATE SET"), "{upsert}");

    let select = od::observed_delta_select_sql(SqlDialect::BigQuery, "ds", "m", "s", "e").unwrap();
    assert!(
        !select.contains("IS NULL") && !select.contains("IS NOT NULL"),
        "absence must be row absence, never a NULL test: {select}"
    );
    assert!(select.contains("WHERE model_name = 'm'"), "{select}");
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

// ── The tombstone ledger's table DDL ───────────────────────────────────────
//
// The *table* is bookkeeping and lives here; every statement the tombstone
// ledger participates in is a maintenance statement, single-owned by
// `smelt_logical::maintenance::emit::succession` and gated by
// `cargo test -p smelt-runtime --test statement_parity`.

fn tombstone_key_cols() -> Vec<(String, DataType)> {
    vec![("customer_id".to_string(), DataType::Integer)]
}

fn tombstone_clock() -> DataType {
    DataType::Timestamp {
        with_timezone: false,
    }
}

/// Exhaustive, and Spark refused by name rather than handed DuckDB DDL.
#[test]
fn the_tombstone_dispatch_is_exhaustive_and_refuses_spark() {
    for dialect in ALL_DIALECTS {
        let ddl = tombstone::tombstone_table_ddl(
            dialect,
            "ds.customer_history__tombstones",
            &tombstone_key_cols(),
            "changed_at",
            &tombstone_clock(),
        );
        let drop = tombstone::tombstone_table_drop_ddl(dialect, "ds.customer_history__tombstones");
        match dialect {
            SqlDialect::SparkSQL => {
                let err = ddl.expect_err("Spark has no tombstone ledger").to_string();
                assert!(err.contains("Spark"), "{err}");
                assert!(err.contains("not realisable"), "{err}");
                drop.expect_err("Spark has no tombstone ledger");
            }
            SqlDialect::DuckDB | SqlDialect::BigQuery => {
                ddl.expect("a realising dialect");
                drop.expect("a realising dialect");
            }
        }
    }
}

/// BigQuery's tombstone DDL carries no DuckDB spelling: no `VARCHAR`, no
/// double-quoted identifier, and a `PRIMARY KEY` that says `NOT ENFORCED`.
#[test]
fn the_bigquery_tombstone_ddl_carries_no_duckdb_spelling() {
    let ddl = tombstone::tombstone_table_ddl(
        SqlDialect::BigQuery,
        "ds.customer_history__tombstones",
        &[("customer_id".to_string(), DataType::Text)],
        "changed_at",
        &tombstone_clock(),
    )
    .expect("BigQuery realises the tombstone ledger");
    assert!(!ddl.contains("VARCHAR"), "{ddl}");
    assert!(!ddl.contains('"'), "{ddl}");
    assert!(ddl.contains("NOT ENFORCED"), "{ddl}");
    assert!(ddl.contains("`ds.customer_history__tombstones`"), "{ddl}");
}

/// DuckDB's path delegates verbatim to the builder it wraps, so the dispatch
/// adds no spelling of its own.
#[test]
fn the_duckdb_tombstone_path_delegates_verbatim() {
    assert_eq!(
        tombstone::tombstone_table_ddl(
            SqlDialect::DuckDB,
            "main.customer_history__tombstones",
            &tombstone_key_cols(),
            "changed_at",
            &tombstone_clock(),
        )
        .expect("DuckDB realises the tombstone ledger"),
        smelt_state::ddl_duckdb::generate_tombstone_table_ddl(
            "main.customer_history__tombstones",
            &tombstone_key_cols(),
            "changed_at",
            &tombstone_clock(),
        )
    );
}

/// A column type with no GoogleSQL spelling is refused with the column named
/// — never substituted, and never an `Unknown` column (`CLAUDE.md`
/// §"Fail-loud discipline").
#[test]
fn an_unmappable_tombstone_column_type_is_refused_with_the_column_named() {
    let err = tombstone::tombstone_table_ddl(
        SqlDialect::BigQuery,
        "ds.t__tombstones",
        &[(
            "payload".to_string(),
            DataType::Map(Box::new(DataType::Text), Box::new(DataType::Text)),
        )],
        "changed_at",
        &tombstone_clock(),
    )
    .expect_err("MAP has no GoogleSQL type")
    .to_string();
    assert!(err.contains("payload"), "{err}");
    assert!(err.contains("MAP"), "{err}");
}

/// Non-vacuity for the two realising dialects, same posture as the ledger's
/// and the observed-delta's.
#[test]
fn the_two_realising_dialects_produce_different_tombstone_ddl() {
    let duckdb = tombstone::tombstone_table_ddl(
        SqlDialect::DuckDB,
        "ds.t__tombstones",
        &tombstone_key_cols(),
        "changed_at",
        &tombstone_clock(),
    )
    .expect("DuckDB");
    let bigquery = tombstone::tombstone_table_ddl(
        SqlDialect::BigQuery,
        "ds.t__tombstones",
        &tombstone_key_cols(),
        "changed_at",
        &tombstone_clock(),
    )
    .expect("BigQuery");
    assert_ne!(duckdb, bigquery);
}
