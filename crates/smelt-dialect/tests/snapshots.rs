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
        smelt_path_ref: None,
        smelt_path_call: None,
        restructure_plans: &[],
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
        smelt_path_ref: None,
        smelt_path_call: None,
        restructure_plans: &[],
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
    let sql = "SELECT * FROM smelt.models.users";
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
    let sql = "SELECT * FROM smelt.models.users";
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
        "SELECT a.id, b.id FROM smelt.models.model_a a JOIN smelt.models.model_b b ON a.id = b.id";
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
    let sql = "SELECT * FROM smelt.models.staging";
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
    let sql = "SELECT * FROM smelt.sources.raw.events";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT * FROM raw.events");
}

// ===== QUALIFY rewrite (Spark) =====

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
fn explode_to_unnest_bigquery() {
    let sql = "SELECT EXPLODE(arr) FROM t";
    let result = print_with(
        sql,
        &SqlDialect::BigQuery,
        &BackendCapabilities::bigquery(),
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
fn every_unchanged_spark() {
    // Spark has no registry emission row for EVERY — it defaults to native.
    let sql = "SELECT EVERY(b) FROM t";
    let result = print_with(
        sql,
        &SqlDialect::SparkSQL,
        &BackendCapabilities::spark(),
        "main",
    );
    insta::assert_snapshot!(result, @"SELECT EVERY(b) FROM t");
}

// ===== Combined rewrites (multiple dialect features in one query) =====

#[test]
fn spark_combined_rewrites() {
    let sql = "SELECT x::INTEGER, ARRAY[1, 2], UNNEST(arr), FROM smelt.models.users WHERE d = DATE '2024-01-01'";
    let result = print_with(
        sql,
        &SqlDialect::SparkSQL,
        &BackendCapabilities::spark(),
        "main",
    );
    insta::assert_snapshot!(result);
}

// ===== SmeltPathRef resolver tests =====

fn make_path_ref_ctx<'a>(
    dialect: &'a SqlDialect,
    caps: &'a BackendCapabilities,
    schema: &'a str,
    resolver: smelt_dialect::SmeltPathRefResolver<'a>,
) -> PrintContext<'a> {
    PrintContext {
        dialect,
        capabilities: caps,
        schema,
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: Some(resolver),
        smelt_path_call: None,
        restructure_plans: &[],
    }
}

fn make_path_call_ctx<'a>(
    dialect: &'a SqlDialect,
    caps: &'a BackendCapabilities,
    schema: &'a str,
    expander: smelt_dialect::SmeltPathCallExpander<'a>,
) -> PrintContext<'a> {
    PrintContext {
        dialect,
        capabilities: caps,
        schema,
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: Some(expander),
        restructure_plans: &[],
    }
}

/// Test 1: path_ref_to_model_emits_schema_qualified_name
/// `smelt.models.users` with resolver returning `main.users` → output is `SELECT * FROM main.users`
#[test]
fn path_ref_to_model_emits_schema_qualified_name() {
    let sql = "SELECT * FROM smelt.models.users";
    let parsed = parse(sql);
    let dialect = SqlDialect::DuckDB;
    let caps = BackendCapabilities::duckdb();
    let resolver: smelt_dialect::SmeltPathRefResolver = Box::new(|segs: &[String]| {
        if segs == ["models", "users"] {
            Some("main.users".to_string())
        } else {
            None
        }
    });
    let ctx = make_path_ref_ctx(&dialect, &caps, "main", resolver);
    let result = print(&parsed.syntax(), &ctx);
    assert_eq!(
        result, "SELECT * FROM main.users",
        "expected schema-qualified name, got: {}",
        result
    );
}

/// Test 2: path_ref_to_seed_emits_seed_table_name
/// `smelt.seeds.raw.users` resolver returns `main.users` (schema-qualified leaf name,
/// consistent with `make_path_ref_resolver` in compiler.rs which uses
/// `format!("{}.{}", schema, rest.join("_"))` → `main.raw_users` for multi-segment seeds
/// or `main.users` for single-segment; the compiler joins all rest segments with '_',
/// so ["seeds", "raw", "users"] → `main.raw_users`).
#[test]
fn path_ref_to_seed_emits_seed_table_name() {
    let sql = "SELECT * FROM smelt.seeds.raw.users";
    let parsed = parse(sql);
    let dialect = SqlDialect::DuckDB;
    let caps = BackendCapabilities::duckdb();
    // Mirrors what `make_path_ref_resolver` emits: schema + rest.join("_")
    let resolver: smelt_dialect::SmeltPathRefResolver = Box::new(|segs: &[String]| {
        if let Some(("seeds", rest)) = segs.split_first().map(|(h, t)| (h.as_str(), t)) {
            if !rest.is_empty() {
                return Some(format!("main.{}", rest.join("_")));
            }
        }
        None
    });
    let ctx = make_path_ref_ctx(&dialect, &caps, "main", resolver);
    let result = print(&parsed.syntax(), &ctx);
    assert!(
        result.contains("main.raw_users"),
        "expected schema-qualified seed table name `main.raw_users`, got: {}",
        result
    );
}

/// Test 3: path_ref_to_source_emits_source_declared_name
/// `smelt.sources.raw.events` resolver returns `raw_events`
#[test]
fn path_ref_to_source_emits_source_declared_name() {
    let sql = "SELECT * FROM smelt.sources.raw.events";
    let parsed = parse(sql);
    let dialect = SqlDialect::DuckDB;
    let caps = BackendCapabilities::duckdb();
    let resolver: smelt_dialect::SmeltPathRefResolver = Box::new(|segs: &[String]| {
        if segs == ["sources", "raw", "events"] {
            Some("raw_events".to_string())
        } else {
            None
        }
    });
    let ctx = make_path_ref_ctx(&dialect, &caps, "main", resolver);
    let result = print(&parsed.syntax(), &ctx);
    assert!(
        result.contains("raw_events"),
        "expected source table name, got: {}",
        result
    );
}

/// Test 4: path_call_emits_expanded_function_body
/// `smelt.functions.patterns.session_rollup(events, 30)` expander returns a SELECT
#[test]
fn path_call_emits_expanded_function_body() {
    let sql = "SELECT * FROM smelt.functions.patterns.session_rollup(events, 30)";
    let parsed = parse(sql);
    let dialect = SqlDialect::DuckDB;
    let caps = BackendCapabilities::duckdb();
    let expander: smelt_dialect::SmeltPathCallExpander = Box::new(
        |segs: &[String], _positional: Vec<String>, _named: Vec<(String, String)>| {
            if segs == ["functions", "patterns", "session_rollup"] {
                Some("SELECT user_id, COUNT(*) AS cnt FROM events GROUP BY user_id".to_string())
            } else {
                None
            }
        },
    );
    let ctx = make_path_call_ctx(&dialect, &caps, "main", expander);
    let result = print(&parsed.syntax(), &ctx);
    assert!(
        result.contains("SELECT user_id"),
        "expected expanded function body, got: {}",
        result
    );
}

/// Test 5: extern_call_unchanged
/// `SELECT read_parquet('foo.parquet')` with no path resolver → byte-identical to input
#[test]
fn extern_call_unchanged() {
    let sql = "SELECT read_parquet('foo.parquet')";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    assert_eq!(
        result, sql,
        "extern call (no smelt extension) must be byte-identical, got: {}",
        result
    );
}

/// Test 6: duckdb_byte_identity_preserved_on_path_form
/// Plain DuckDB SQL (no smelt extensions) must be byte-identical through the printer
#[test]
fn duckdb_byte_identity_preserved_on_path_form() {
    let sql = "SELECT id FROM main.users WHERE id > 1";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    assert_eq!(
        result, sql,
        "plain DuckDB SQL must be byte-identical, got: {}",
        result
    );
}

/// A rename must not fire when the author already wrote the target spelling.
///
/// `json_extract_string` is DuckDB's own name for `JSON_EXTRACT_TEXT`, carried
/// as an alias on that entry — so the canonical entry's DuckDB `Rename` would
/// otherwise rewrite the user's text into different case, breaking the
/// byte-identity `architecture.md` promises for DuckDB-flavoured input.
#[test]
fn a_rename_is_suppressed_when_the_source_already_uses_the_target_spelling() {
    let sql = "SELECT json_extract_string(payload, '$.k') AS k FROM events";
    let result = print_with(
        sql,
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    assert_eq!(result, sql, "the author's own DuckDB spelling must survive");

    // …and the rename still fires for a spelling that is not the target.
    let renamed = print_with(
        "SELECT EVERY(flag) AS a FROM events",
        &SqlDialect::DuckDB,
        &BackendCapabilities::duckdb(),
        "main",
    );
    assert!(renamed.contains("BOOL_AND("), "{renamed}");
}
