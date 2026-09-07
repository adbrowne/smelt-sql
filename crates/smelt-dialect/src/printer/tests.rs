//! Printer unit tests: identity, ref resolution, capability-gated
//! rewrites, and registry-driven emission.

use super::*;
use smelt_parser::parse;

fn duckdb_ctx() -> (SqlDialect, BackendCapabilities) {
    let caps = BackendCapabilities::duckdb();
    (caps.dialect, caps)
}

fn spark_ctx() -> (SqlDialect, BackendCapabilities) {
    let caps = BackendCapabilities::spark();
    (caps.dialect, caps)
}

fn bigquery_ctx() -> (SqlDialect, BackendCapabilities) {
    let caps = BackendCapabilities::bigquery();
    (caps.dialect, caps)
}

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
        settled_emissions: &[],
    };
    print(&parsed.syntax(), &ctx)
}

// ===== Identity tests =====

#[test]
fn test_identity_simple_select() {
    let sql = "SELECT * FROM users";
    let (d, c) = duckdb_ctx();
    assert_eq!(print_with(sql, &d, &c, "main"), sql);
}

#[test]
fn test_identity_complex_query() {
    let sql = "SELECT u.id, COUNT(*) AS cnt\nFROM users u\nWHERE u.active = 1\nGROUP BY u.id\nHAVING COUNT(*) > 5\nORDER BY cnt DESC\nLIMIT 10";
    let (d, c) = duckdb_ctx();
    assert_eq!(print_with(sql, &d, &c, "main"), sql);
}

#[test]
fn test_identity_with_comments() {
    let sql = "-- This is a comment\nSELECT * FROM users";
    let (d, c) = duckdb_ctx();
    assert_eq!(print_with(sql, &d, &c, "main"), sql);
}

#[test]
fn test_identity_with_cte() {
    let sql = "WITH active AS (SELECT * FROM users WHERE active = 1) SELECT * FROM active";
    let (d, c) = duckdb_ctx();
    assert_eq!(print_with(sql, &d, &c, "main"), sql);
}

#[test]
fn test_identity_preserves_whitespace() {
    let sql =
        "SELECT\n    user_id,\n    COUNT(*) AS count\nFROM events\nWHERE event_type = 'click'";
    let (d, c) = duckdb_ctx();
    assert_eq!(print_with(sql, &d, &c, "main"), sql);
}

#[test]
fn test_identity_preserves_doubled_quote_in_string_literal() {
    // SQL standard '' escape inside a single-quoted string must round-trip
    // verbatim so DuckDB sees the same literal it was given.
    let sql = "SELECT CASE WHEN x > 0 THEN 'Can''t Lose Them' ELSE 'Other' END AS label FROM t";
    let (d, c) = duckdb_ctx();
    assert_eq!(print_with(sql, &d, &c, "main"), sql);
}

// ===== Ref resolution tests =====

#[test]
fn test_ref_resolution() {
    let sql = "SELECT * FROM smelt.models.users";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, "SELECT * FROM main.users");
}

#[test]
fn test_ref_resolution_custom_schema() {
    let sql = "SELECT * FROM smelt.models.users";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "analytics");
    assert_eq!(result, "SELECT * FROM analytics.users");
}

#[test]
fn test_multiple_refs() {
    let sql =
        "SELECT a.id, b.id FROM smelt.models.model_a a JOIN smelt.models.model_b b ON a.id = b.id";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert!(result.contains("main.model_a"));
    assert!(result.contains("main.model_b"));
    assert!(!result.contains("smelt.ref"));
}

// ===== Cross-engine ref resolution tests =====

#[test]
fn test_cross_engine_ref_resolution() {
    let sql = "SELECT * FROM smelt.models.spark_model";
    let parsed = parse(sql);
    let (d, c) = duckdb_ctx();
    let mut cross_refs = HashMap::new();
    cross_refs.insert(
        "spark_model".to_string(),
        "read_parquet('/data/warehouse/default/spark_model/**/*.parquet', hive_partitioning=true)"
            .to_string(),
    );
    let ctx = PrintContext {
        dialect: &d,
        capabilities: &c,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: cross_refs,
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: None,
        restructure_plans: &[],
        settled_emissions: &[],
    };
    let result = print(&parsed.syntax(), &ctx);
    assert!(
        result.contains("read_parquet("),
        "Expected read_parquet, got: {}",
        result
    );
    assert!(
        result.contains("spark_model/**/*.parquet"),
        "Expected parquet glob path, got: {}",
        result
    );
    assert!(
        !result.contains("main.spark_model"),
        "Should not contain schema-qualified ref, got: {}",
        result
    );
}

#[test]
fn test_cross_engine_ref_mixed_with_normal_refs() {
    let sql = "SELECT a.id, b.id FROM smelt.models.local_model a JOIN smelt.models.spark_model b ON a.id = b.id";
    let parsed = parse(sql);
    let (d, c) = duckdb_ctx();
    let mut cross_refs = HashMap::new();
    cross_refs.insert(
        "spark_model".to_string(),
        "read_parquet('/data/spark_model/**/*.parquet', hive_partitioning=true)".to_string(),
    );
    let ctx = PrintContext {
        dialect: &d,
        capabilities: &c,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: cross_refs,
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: None,
        restructure_plans: &[],
        settled_emissions: &[],
    };
    let result = print(&parsed.syntax(), &ctx);
    assert!(
        result.contains("main.local_model"),
        "Normal ref should resolve to schema.model, got: {}",
        result
    );
    assert!(
        result.contains("read_parquet("),
        "Cross-engine ref should resolve to read_parquet, got: {}",
        result
    );
}

// ===== Source resolution tests =====

#[test]
fn test_source_resolution() {
    let sql = "SELECT * FROM smelt.sources.raw.events";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, "SELECT * FROM raw.events");
}

// ===== Formatting/whitespace preservation =====

#[test]
fn test_ref_preserves_surrounding_formatting() {
    let sql = "SELECT\n    user_id,\n    COUNT(*) as count\nFROM smelt.models.events\nWHERE event_type = 'click'";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert!(result.contains("SELECT\n    user_id,"));
    assert!(result.contains("FROM main.events"));
    assert!(result.contains("\nWHERE event_type = 'click'"));
}

// ===== QUALIFY rewrite tests =====

#[test]
fn test_qualify_rewrite_spark() {
    let sql = "SELECT *, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t QUALIFY rn = 1";
    let (d, c) = spark_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert!(
        result.contains("SELECT * FROM ("),
        "Expected subquery wrapper, got: {}",
        result
    );
    assert!(
        result.contains("WHERE rn = 1"),
        "Expected WHERE clause, got: {}",
        result
    );
    assert!(
        !result.contains("QUALIFY"),
        "QUALIFY should be removed, got: {}",
        result
    );
}

#[test]
fn test_qualify_no_rewrite_duckdb() {
    let sql = "SELECT *, ROW_NUMBER() OVER (ORDER BY id) AS rn FROM t QUALIFY rn = 1";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert!(result.contains("QUALIFY"), "DuckDB should preserve QUALIFY");
    assert_eq!(result, sql);
}

// ===== ARRAY literal rewrite tests =====

#[test]
fn test_array_rewrite_spark() {
    let sql = "SELECT ARRAY[1, 2, 3] FROM t";
    let (d, c) = spark_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert!(
        result.contains("ARRAY(1, 2, 3)"),
        "Expected ARRAY() syntax, got: {}",
        result
    );
    assert!(
        !result.contains('['),
        "Brackets should be replaced, got: {}",
        result
    );
}

#[test]
fn test_array_no_rewrite_duckdb() {
    let sql = "SELECT ARRAY[1, 2, 3] FROM t";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, sql);
}

// ===== DATE literal rewrite tests =====

#[test]
fn test_date_rewrite_spark() {
    let sql = "SELECT * FROM t WHERE d = DATE '2024-01-01'";
    let (d, c) = spark_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert!(
        result.contains("DATE('2024-01-01')"),
        "Expected DATE() function syntax, got: {}",
        result
    );
}

#[test]
fn test_date_no_rewrite_duckdb() {
    let sql = "SELECT * FROM t WHERE d = DATE '2024-01-01'";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, sql);
}

// ===== :: cast rewrite tests =====

#[test]
fn test_double_colon_rewrite_spark() {
    let sql = "SELECT x::INTEGER FROM t";
    let (d, c) = spark_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, "SELECT CAST(x AS INTEGER) FROM t");
}

#[test]
fn test_double_colon_no_rewrite_duckdb() {
    let sql = "SELECT x::INTEGER FROM t";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, sql);
}

#[test]
fn test_cast_function_passthrough_spark() {
    let sql = "SELECT CAST(x AS INTEGER) FROM t";
    let (d, c) = spark_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, sql);
}

#[test]
fn test_double_colon_varchar_rewrite_spark() {
    let sql = "SELECT name::VARCHAR FROM t";
    let (d, c) = spark_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, "SELECT CAST(name AS VARCHAR) FROM t");
}

// ===== Trailing comma removal tests =====

#[test]
fn test_trailing_comma_stripped_spark() {
    let sql = "SELECT a, b, FROM t";
    let (d, c) = spark_ctx();
    let result = print_with(sql, &d, &c, "main");
    // The comma is removed; whitespace around it is preserved
    assert!(!result.contains("b,"), "Trailing comma should be removed");
    assert!(result.contains("a, b"), "Non-trailing commas preserved");
    assert!(!result.contains(", FROM"), "Comma before FROM removed");
}

#[test]
fn test_trailing_comma_preserved_duckdb() {
    let sql = "SELECT a, b, FROM t";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, sql);
}

#[test]
fn test_group_by_trailing_comma_stripped_spark() {
    let sql = "SELECT a, COUNT(*) FROM t GROUP BY a,";
    let (d, c) = spark_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, "SELECT a, COUNT(*) FROM t GROUP BY a");
}

#[test]
fn test_no_trailing_comma_unchanged_spark() {
    let sql = "SELECT a, b FROM t";
    let (d, c) = spark_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, sql);
}

// ===== EXPLODE/UNNEST renaming tests =====

#[test]
fn test_explode_to_unnest_duckdb() {
    let sql = "SELECT EXPLODE(arr) FROM t";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, "SELECT UNNEST(arr) FROM t");
}

#[test]
fn test_unnest_to_explode_spark() {
    let sql = "SELECT UNNEST(arr) FROM t";
    let (d, c) = spark_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, "SELECT EXPLODE(arr) FROM t");
}

#[test]
fn test_explode_unchanged_spark() {
    let sql = "SELECT EXPLODE(arr) FROM t";
    let (d, c) = spark_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, sql);
}

#[test]
fn test_unnest_unchanged_duckdb() {
    let sql = "SELECT UNNEST(arr) FROM t";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, sql);
}

#[test]
fn test_explode_to_unnest_bigquery() {
    let sql = "SELECT EXPLODE(arr) FROM t";
    let (d, c) = bigquery_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, "SELECT UNNEST(arr) FROM t");
}

// ===== EVERY/BOOL_AND/BOOL_OR remapping tests =====

#[test]
fn test_every_to_bool_and_duckdb() {
    let sql = "SELECT EVERY(b) FROM t";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, "SELECT BOOL_AND(b) FROM t");
}

#[test]
fn test_every_unchanged_spark() {
    // Spark has no registry emission row for EVERY — it defaults to
    // native, unlike DuckDB (BOOL_AND) and BigQuery (LOGICAL_AND).
    let sql = "SELECT EVERY(b) FROM t";
    let (d, c) = spark_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, sql);
}

#[test]
fn test_bool_and_to_every_spark() {
    let sql = "SELECT BOOL_AND(b) FROM t";
    let (d, c) = spark_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, "SELECT EVERY(b) FROM t");
}

#[test]
fn test_bool_or_to_some_spark() {
    let sql = "SELECT BOOL_OR(b) FROM t";
    let (d, c) = spark_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, "SELECT SOME(b) FROM t");
}

#[test]
fn test_bool_and_unchanged_duckdb() {
    let sql = "SELECT BOOL_AND(b) FROM t";
    let (d, c) = duckdb_ctx();
    let result = print_with(sql, &d, &c, "main");
    assert_eq!(result, sql);
}

// ===== struct-returning call .* projection tests =====

#[test]
fn struct_returning_call_dot_star_lowers_to_field_projections() {
    let sql = "SELECT smelt.functions.parse_event_payload(payload).* FROM e";
    let parsed = parse(sql);
    let (d, c) = duckdb_ctx();
    // The expander returns a brace-struct literal body with three fields.
    let body =
        "{json_extract_string(payload, '$.event_name') AS event_name, json_extract_string(payload, '$.platform') AS platform, json_extract_string(payload, '$.url') AS url}";
    let ctx = PrintContext {
        dialect: &d,
        capabilities: &c,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: Some(Box::new(move |_segs, _pos, _named| Some(body.to_string()))),
        restructure_plans: &[],
        settled_emissions: &[],
    };
    let result = print(&parsed.syntax(), &ctx);
    // Should expand to three separate aliased projections, not the struct literal
    assert!(
        result.contains("json_extract_string(payload, '$.event_name') AS event_name"),
        "expected event_name projection, got: {result}"
    );
    assert!(
        result.contains("json_extract_string(payload, '$.platform') AS platform"),
        "expected platform projection, got: {result}"
    );
    assert!(
        result.contains("json_extract_string(payload, '$.url') AS url"),
        "expected url projection, got: {result}"
    );
    // Should NOT contain the brace-struct literal syntax (curly braces in the output)
    assert!(
        !result.contains('{'),
        "output should not contain struct literal braces, got: {result}"
    );
    assert!(
        !result.contains(".*"),
        "output should not contain .*, got: {result}"
    );
}

// ===== FROM-position derived-table alias synthesis tests =====

/// Parse `SELECT * FROM smelt.functions.sessionize(x)` (no explicit alias).
/// The printer must synthesise a `__smelt_t<N>` alias so DuckDB accepts the
/// derived table.  The expanded body is a SELECT statement.
#[test]
fn table_expr_call_in_from_without_alias_synthesises_one() {
    let sql = "SELECT * FROM smelt.functions.sessionize(x)";
    let parsed = parse(sql);
    let (d, c) = duckdb_ctx();
    let body = "SELECT * FROM some_table WHERE x = 1";
    let ctx = PrintContext {
        dialect: &d,
        capabilities: &c,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: Some(Box::new(move |_segs, _pos, _named| Some(body.to_string()))),
        restructure_plans: &[],
        settled_emissions: &[],
    };
    let result = print(&parsed.syntax(), &ctx);
    // Must contain a synthesised alias beginning with __smelt_t
    assert!(
        result.contains("__smelt_t"),
        "expected synthesised __smelt_t alias, got: {result}"
    );
    // Expanded body must appear inside parentheses
    assert!(
        result.contains("(SELECT * FROM some_table WHERE x = 1)"),
        "expected expanded body in parens, got: {result}"
    );
    // The alias must follow the closing paren
    assert!(
        result.contains(") AS __smelt_t"),
        "expected ) AS __smelt_t<N>, got: {result}"
    );
}

/// Same input but with `AS s` after the call.  The printer must use the
/// user's alias verbatim and must not emit a synthesised `__smelt_t` alias.
#[test]
fn table_expr_call_in_from_with_alias_passes_through() {
    let sql = "SELECT * FROM smelt.functions.sessionize(x) AS s";
    let parsed = parse(sql);
    let (d, c) = duckdb_ctx();
    let body = "SELECT * FROM some_table WHERE x = 1";
    let ctx = PrintContext {
        dialect: &d,
        capabilities: &c,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: Some(Box::new(move |_segs, _pos, _named| Some(body.to_string()))),
        restructure_plans: &[],
        settled_emissions: &[],
    };
    let result = print(&parsed.syntax(), &ctx);
    // Must NOT synthesise a __smelt_t alias
    assert!(
        !result.contains("__smelt_t"),
        "should not synthesise alias when user supplied one, got: {result}"
    );
    // The expanded body must still appear
    assert!(
        result.contains("SELECT * FROM some_table WHERE x = 1"),
        "expected expanded body in output, got: {result}"
    );
    // The user-supplied alias must appear
    assert!(
        result.contains("AS s"),
        "expected user alias AS s, got: {result}"
    );
}

/// A `smelt.<path>(args)` call in SELECT-list position (not FROM) must NOT
/// get wrapped in `(...) AS __smelt_t<N>`.  The printer should expand the
/// body verbatim (or the position is structurally unreachable for
/// TableExpr-returning calls — either way no alias synthesis occurs).
#[test]
fn table_expr_call_in_select_list_does_not_synthesise_alias() {
    // In a SELECT list the path call is inside EXPRESSION/SELECT_ITEM, not TABLE_REF.
    // Even if the expander returns a SELECT body, no alias wrapping should occur.
    let sql = "SELECT smelt.functions.some_fn(x) FROM t";
    let parsed = parse(sql);
    let (d, c) = duckdb_ctx();
    let body = "some_expression";
    let ctx = PrintContext {
        dialect: &d,
        capabilities: &c,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: Some(Box::new(move |_segs, _pos, _named| Some(body.to_string()))),
        restructure_plans: &[],
        settled_emissions: &[],
    };
    let result = print(&parsed.syntax(), &ctx);
    // Must NOT synthesise a __smelt_t alias in SELECT list position
    assert!(
        !result.contains("__smelt_t"),
        "should not synthesise alias in SELECT list position, got: {result}"
    );
    // The expanded body must appear verbatim
    assert!(
        result.contains("some_expression"),
        "expected expanded body, got: {result}"
    );
}

// ===== Nested smelt.define fixpoint tests (BUG-013) =====

/// Printing a model SQL that calls `smelt.functions.outer(x)` where `outer`
/// expands to `(smelt.functions.inner(x))` and `inner` expands to `(x + 1)`
/// must produce output containing `(x + 1)` (not `smelt.functions.inner`).
///
/// Before the fix, the reparse step did not produce `SMELT_PATH_CALL` nodes
/// from a bare/parenthesised fragment, so the inner path-call was passed
/// through verbatim to DuckDB → `Catalog "smelt" does not exist`.
#[test]
fn nested_define_chain_expands_to_fixpoint() {
    // Two-level chain: wrap_fn → (smelt.functions.increment(x)), increment → (x + 1).
    // "wrap_fn" and "increment" are plain identifiers with no SQL keyword
    // collision, so the parser reliably recognises the call as SMELT_PATH_CALL.
    let sql = "SELECT smelt.functions.wrap_fn(x) FROM t";
    let parsed = parse(sql);
    let (d, c) = duckdb_ctx();

    let wrap_body = "(smelt.functions.increment(x))";
    let increment_body = "(x + 1)";
    let ctx = PrintContext {
        dialect: &d,
        capabilities: &c,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        smelt_path_call: Some(Box::new(move |segs, _pos, _named| {
            match segs.last().map(|s| s.as_str()) {
                Some("wrap_fn") => Some(wrap_body.to_string()),
                Some("increment") => Some(increment_body.to_string()),
                _ => None,
            }
        })),
        restructure_plans: &[],
        settled_emissions: &[],
    };
    let result = print(&parsed.syntax(), &ctx);

    // The final output must contain the fully expanded arithmetic, not any
    // residual smelt.functions.* reference.
    assert!(
        !result.contains("smelt.functions"),
        "nested smelt.functions.* must be fully expanded, got: {result}"
    );
    assert!(
        result.contains("x + 1"),
        "expected fully expanded body '(x + 1)', got: {result}"
    );
}

/// BUG-018: a block `PASSING <name> AS (<body>)` clause must arrive at the
/// expander as a named binding (`<name>` → `<body>`), so a fragment
/// parameter supplied via PASSING is substituted rather than falling back
/// to its default.
#[test]
fn block_passing_binds_fragment_into_named_args() {
    // `metrics` is supplied via a trailing PASSING block, not inline.
    let sql = "SELECT * FROM smelt.functions.rollup(t) PASSING metrics AS (COUNT(*))";
    let parsed = parse(sql);
    let (d, c) = duckdb_ctx();

    let ctx = PrintContext {
        dialect: &d,
        capabilities: &c,
        schema: "main",
        ephemeral_models: HashSet::new(),
        cross_engine_refs: HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
        smelt_path_ref: None,
        // The expander mimics body substitution: it reads the `metrics`
        // binding out of `named` and splices it into the body. If the
        // PASSING clause were ignored, `metrics` would fall back to `()`.
        smelt_path_call: Some(Box::new(move |_segs, _pos, named| {
            let metrics = named
                .iter()
                .find(|(k, _)| k == "metrics")
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| "()".to_string());
            Some(format!(
                "(SELECT group_col, {metrics} FROM t GROUP BY group_col)"
            ))
        })),
        restructure_plans: &[],
        settled_emissions: &[],
    };
    let result = print(&parsed.syntax(), &ctx);

    assert!(
        result.contains("COUNT(*)"),
        "PASSING body must be bound to `metrics`, got: {result}"
    );
    assert!(
        !result.contains("group_col, ()"),
        "`metrics` must not fall back to its default `()`, got: {result}"
    );
}
