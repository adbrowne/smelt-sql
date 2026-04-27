use std::collections::{HashMap, HashSet};

use smelt_dialect::{print, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::parse;

fn print_with(sql: &str, dialect: &SqlDialect, caps: &BackendCapabilities, schema: &str) -> String {
    let parsed = parse(sql);
    let ctx = PrintContext {
        dialect,
        capabilities: caps,
        schema,
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
    };
    print(&parsed.syntax(), &ctx)
}

fn print_with_ephemerals(
    sql: &str,
    dialect: &SqlDialect,
    caps: &BackendCapabilities,
    schema: &str,
    ephemerals: &[&str],
) -> String {
    let parsed = parse(sql);
    let ctx = PrintContext {
        dialect,
        capabilities: caps,
        schema,
        ephemeral_models: ephemerals.iter().copied().collect(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
    };
    print(&parsed.syntax(), &ctx)
}

// ===== DuckDB identity (verbatim passthrough) =====

#[test]
fn duckdb_identity_simple() {
    let sql = "SELECT * FROM users";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT * FROM users");
}

#[test]
fn duckdb_identity_complex() {
    let sql = "SELECT u.id, COUNT(*) AS cnt\nFROM users u\nWHERE u.active = 1\nGROUP BY u.id\nHAVING COUNT(*) > 5\nORDER BY cnt DESC\nLIMIT 10";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    insta::assert_snapshot!(result);
}

#[test]
fn duckdb_identity_with_cte() {
    let sql = "WITH active AS (SELECT * FROM users WHERE active = 1) SELECT * FROM active";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    insta::assert_snapshot!(result, @"WITH active AS (SELECT * FROM users WHERE active = 1) SELECT * FROM active");
}

// ===== Ref resolution =====

#[test]
fn ref_resolution_duckdb() {
    let sql = "SELECT * FROM smelt.ref('users')";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT * FROM main.users");
}

#[test]
fn ref_resolution_custom_schema() {
    let sql = "SELECT * FROM smelt.ref('users')";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "analytics",
    );
    insta::assert_snapshot!(result, @"SELECT * FROM analytics.users");
}

#[test]
fn ref_resolution_multiple() {
    let sql =
        "SELECT a.id, b.id FROM smelt.ref('model_a') a JOIN smelt.ref('model_b') b ON a.id = b.id";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    insta::assert_snapshot!(result);
}

#[test]
fn ref_resolution_ephemeral() {
    let sql = "SELECT * FROM smelt.ref('staging')";
    let result = print_with_ephemerals(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
        &["staging"],
    );
    insta::assert_snapshot!(result, @"SELECT * FROM __smelt_staging");
}

// ===== Source resolution =====

#[test]
fn source_resolution() {
    let sql = "SELECT * FROM smelt.source('raw.events')";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT * FROM raw.events");
}

// ===== QUALIFY rewrite (PostgreSQL/Spark) =====

#[test]
fn qualify_rewrite_postgresql() {
    let sql = "SELECT *, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t QUALIFY rn = 1";
    let result = print_with(
        sql,
        &SqlDialect::PostgreSQL,
        &BackendCapabilities::postgresql(),
        "main",
    );
    insta::assert_snapshot!(result);
}

#[test]
fn qualify_rewrite_spark() {
    let sql = "SELECT *, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t QUALIFY rn = 1";
    let result = print_with(
        sql,
        &SqlDialect::SparkSQL,
        &BackendCapabilities::spark(),
        "main",
    );
    insta::assert_snapshot!(result);
}

#[test]
fn qualify_preserved_duckdb() {
    let sql = "SELECT *, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t QUALIFY rn = 1";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT *, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t QUALIFY rn = 1");
}

// ===== ARRAY literal rewrite (Spark) =====

#[test]
fn array_rewrite_spark() {
    let sql = "SELECT ARRAY[1, 2, 3] FROM t";
    let result = print_with(
        sql,
        &SqlDialect::SparkSQL,
        &BackendCapabilities::spark(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT ARRAY(1, 2, 3) FROM t");
}

#[test]
fn array_preserved_duckdb() {
    let sql = "SELECT ARRAY[1, 2, 3] FROM t";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT ARRAY[1, 2, 3] FROM t");
}

// ===== DATE literal rewrite (Spark) =====

#[test]
fn date_rewrite_spark() {
    let sql = "SELECT * FROM t WHERE d = DATE '2024-01-01'";
    let result = print_with(
        sql,
        &SqlDialect::SparkSQL,
        &BackendCapabilities::spark(),
        "main",
    );
    insta::assert_snapshot!(result);
}

#[test]
fn date_preserved_duckdb() {
    let sql = "SELECT * FROM t WHERE d = DATE '2024-01-01'";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT * FROM t WHERE d = DATE '2024-01-01'");
}

// ===== :: cast rewrite (Spark) =====

#[test]
fn double_colon_cast_rewrite_spark() {
    let sql = "SELECT x::INTEGER FROM t";
    let result = print_with(
        sql,
        &SqlDialect::SparkSQL,
        &BackendCapabilities::spark(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT CAST(x AS INTEGER) FROM t");
}

#[test]
fn double_colon_varchar_rewrite_spark() {
    let sql = "SELECT name::VARCHAR FROM t";
    let result = print_with(
        sql,
        &SqlDialect::SparkSQL,
        &BackendCapabilities::spark(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT CAST(name AS VARCHAR) FROM t");
}

#[test]
fn cast_function_passthrough_spark() {
    let sql = "SELECT CAST(x AS INTEGER) FROM t";
    let result = print_with(
        sql,
        &SqlDialect::SparkSQL,
        &BackendCapabilities::spark(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT CAST(x AS INTEGER) FROM t");
}

#[test]
fn double_colon_preserved_duckdb() {
    let sql = "SELECT x::INTEGER FROM t";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT x::INTEGER FROM t");
}

// ===== Trailing comma removal (Spark) =====

#[test]
fn trailing_comma_stripped_spark() {
    let sql = "SELECT a, b, FROM t";
    let result = print_with(
        sql,
        &SqlDialect::SparkSQL,
        &BackendCapabilities::spark(),
        "main",
    );
    insta::assert_snapshot!(result);
}

#[test]
fn group_by_trailing_comma_stripped_spark() {
    let sql = "SELECT a, COUNT(*) FROM t GROUP BY a,";
    let result = print_with(
        sql,
        &SqlDialect::SparkSQL,
        &BackendCapabilities::spark(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT a, COUNT(*) FROM t GROUP BY a");
}

#[test]
fn trailing_comma_preserved_duckdb() {
    let sql = "SELECT a, b, FROM t";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT a, b, FROM t");
}

// ===== Function remapping =====

#[test]
fn explode_to_unnest_duckdb() {
    let sql = "SELECT EXPLODE(arr) FROM t";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT UNNEST(arr) FROM t");
}

#[test]
fn unnest_to_explode_spark() {
    let sql = "SELECT UNNEST(arr) FROM t";
    let result = print_with(
        sql,
        &SqlDialect::SparkSQL,
        &BackendCapabilities::spark(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT EXPLODE(arr) FROM t");
}

#[test]
fn explode_to_unnest_postgresql() {
    let sql = "SELECT EXPLODE(arr) FROM t";
    let result = print_with(
        sql,
        &SqlDialect::PostgreSQL,
        &BackendCapabilities::postgresql(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT UNNEST(arr) FROM t");
}

#[test]
fn every_to_bool_and_duckdb() {
    let sql = "SELECT EVERY(b) FROM t";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT BOOL_AND(b) FROM t");
}

#[test]
fn bool_and_to_every_spark() {
    let sql = "SELECT BOOL_AND(b) FROM t";
    let result = print_with(
        sql,
        &SqlDialect::SparkSQL,
        &BackendCapabilities::spark(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT EVERY(b) FROM t");
}

#[test]
fn bool_or_to_some_spark() {
    let sql = "SELECT BOOL_OR(b) FROM t";
    let result = print_with(
        sql,
        &SqlDialect::SparkSQL,
        &BackendCapabilities::spark(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT SOME(b) FROM t");
}

#[test]
fn every_unchanged_postgresql() {
    let sql = "SELECT EVERY(b) FROM t";
    let result = print_with(
        sql,
        &SqlDialect::PostgreSQL,
        &BackendCapabilities::postgresql(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT EVERY(b) FROM t");
}

// ===== Combined rewrites (multiple dialect features in one query) =====

#[test]
fn spark_combined_rewrites() {
    let sql = "SELECT x::INTEGER, ARRAY[1, 2], UNNEST(arr), FROM smelt.ref('users') WHERE d = DATE '2024-01-01'";
    let result = print_with(
        sql,
        &SqlDialect::SparkSQL,
        &BackendCapabilities::spark(),
        "main",
    );
    insta::assert_snapshot!(result);
}
