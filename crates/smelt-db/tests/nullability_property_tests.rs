//! Value-based nullability soundness tests.
//!
//! The soundness contract: `nullable: false` is a hard guarantee — the column cannot contain
//! NULL in any row, for any input data satisfying the declared source schemas.
//! `nullable: true` means only "may contain NULL".
//!
//! These tests are value-based: we generate queries over tables with actual NULL values and
//! assert that no column smelt infers as `nullable: false` ever contains a NULL in DuckDB results.
//!
//! Strategy:
//!   - Generate a table schema with some nullable and some non-nullable columns.
//!   - Insert rows with high NULL density for nullable columns, zero NULLs for non-nullable.
//!   - Generate a single-table SELECT query.
//!   - Run smelt inference → collect which columns are `nullable: false`.
//!   - Run the query on DuckDB → for each `nullable: false` column, assert null_count == 0.

#[allow(dead_code)]
mod prop_helpers;

use prop_helpers::duckdb_oracle::DuckDbOracle;
use prop_helpers::generators::{
    assemble_cte_query, column_pool_strategy, generate_expr, test_scenario_strategy, QueryShape,
    TypedExpr, TypedSource,
};
use prop_helpers::null_data::{
    build_join_cte_query, build_join_real_table_query, build_null_bearing_query,
    build_setop_mixed_nullability_cte, build_setop_uniform_nonnullable_cte,
    check_nullability_soundness, smoke_coalesce_non_nullable_setup,
    smoke_nullable_passthrough_setup, JoinKind,
};

use smelt_db::apply_outer_join_nullability;
use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_parser::ast::File;
use smelt_types::{DataType, TypedColumn};

use proptest::prelude::*;

// ---- Smoke tests (deterministic) ----

/// Smoke: `COALESCE(nullable_col, 0)` infers non-nullable and DuckDB returns no NULLs.
///
/// This proves the harness can pass the soundness check (green path).
#[test]
fn smoke_coalesce_non_nullable_holds() {
    let oracle = DuckDbOracle::new();
    let (setup_sql, check_sql, expected_nullable) = smoke_coalesce_non_nullable_setup();

    // Execute setup (CREATE TABLE + INSERT)
    oracle
        .execute_ddl(&setup_sql)
        .expect("setup DDL should succeed");

    // Check nullability soundness
    let violations = check_nullability_soundness(&oracle, &check_sql, &expected_nullable).unwrap();
    assert!(
        violations.is_empty(),
        "COALESCE(nullable_col, 0) should be non-nullable but DuckDB observed NULLs: {:?}",
        violations
    );
}

/// Smoke: a nullable column projected as-is stays `nullable: true` and DuckDB results contain NULLs.
///
/// This proves the NULL-bearing data generation actually works — the test MUST observe real NULLs.
#[test]
fn smoke_nullable_column_passthrough() {
    let oracle = DuckDbOracle::new();
    let (setup_sql, check_sql, col_name) = smoke_nullable_passthrough_setup();

    // Execute setup (CREATE TABLE + INSERT)
    oracle
        .execute_ddl(&setup_sql)
        .expect("setup DDL should succeed");

    // Run the query and verify we actually observe NULLs
    let observed_nulls = oracle
        .count_nulls_per_column(&check_sql)
        .expect("query should succeed");

    let null_count = observed_nulls
        .iter()
        .find(|(name, _)| name == &col_name)
        .map(|(_, count)| *count)
        .unwrap_or(0);

    assert!(
        null_count > 0,
        "nullable column passthrough must observe actual NULLs in DuckDB results, got 0. \
         This means NULL data generation is broken.",
    );
}

// ---- Regression tests (added before corresponding fixes) ----

/// Regression: `nullable_col = nullable_col` must infer nullable.
///
/// Comparison operators are NULL-propagating: `NULL = NULL` evaluates to NULL in SQL.
/// The only way the result is guaranteed non-null is if both operands are non-nullable.
/// Prior to the fix, smelt unconditionally returned `nullable: false` for `=`, `<>`, etc.
#[test]
fn regression_comparison_of_nullable_columns_is_nullable() {
    use smelt_db::type_inference::{infer_select_column_types, TypeContext};
    use smelt_parser::ast::File;
    use smelt_types::TypedColumn;

    // Minimal failing SQL from the oracle: bool_col = bool_col where bool_col is nullable.
    let sql = "WITH data AS (SELECT CAST(TRUE AS BOOLEAN) AS bool_col_0) \
               SELECT bool_col_0 = bool_col_0 AS expr_0 FROM data";

    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt");

    let mut ctx = TypeContext::new();
    ctx.add_cte_column(
        "data",
        "bool_col_0",
        TypedColumn::nullable(smelt_types::DataType::Boolean),
    );

    let inferred = infer_select_column_types(&select_stmt, &ctx);
    assert_eq!(inferred.len(), 1, "expected 1 output column");
    assert!(
        inferred[0].nullable,
        "comparison of nullable columns must be nullable (NULL propagates through =), \
         got nullable: false"
    );
}

/// Regression: DuckDB value oracle confirms comparison on nullable INT columns returns NULLs.
#[test]
fn regression_comparison_nullable_int_duckdb_value_check() {
    let oracle = DuckDbOracle::new();
    oracle
        .execute_ddl(
            "CREATE TABLE tbl (x INTEGER);\
             INSERT INTO tbl VALUES (NULL), (1), (NULL), (2)",
        )
        .unwrap();
    let nulls = oracle
        .count_nulls_per_column("SELECT x = x AS result FROM tbl")
        .unwrap();
    assert_eq!(nulls.len(), 1);
    assert!(
        nulls[0].1 > 0,
        "x = x where x is nullable must produce NULLs, got {} NULLs",
        nulls[0].1
    );
}

/// Regression: a source column declared `nullable: false` that appears on the
/// null-supplying side of a LEFT JOIN must infer as `nullable: true` in the
/// output schema.
///
/// Without the outer-join nullability pass (`apply_outer_join_nullability`),
/// `add_source_info_to_type_context` writes the declared `nullable: false` value
/// into the context AFTER `process_from_clause_pure` seeds it, so a naive inline
/// mark would be silently overwritten. This test exercises the real source-column
/// path where the overwrite trap can occur.
///
/// Fixture: `examples/test_workspace/models/users_with_latest_event.sql`
/// LEFT JOINs `smelt.sources.source.events` (right side, `event_id: nullable: false`).
/// After the pass, `event_id` must infer `nullable: true`.
///
/// Two-part check:
///   1. Inference check — smelt marks `event_id` nullable.
///   2. DuckDB value check — a LEFT JOIN with no matching rows produces NULLs
///      for `event_id`, confirming the semantic guarantee is correct.
#[test]
fn regression_left_join_declared_not_null() {
    // ── Part 1: inference check ──────────────────────────────────────────────
    //
    // Simulate the Salsa type_context path in pure Rust:
    //   1. Start with source columns seeded as nullable:true (as build_type_context does).
    //   2. Then overwrite with declared nullable:false (as add_source_info_to_type_context does).
    //   3. Then apply apply_outer_join_nullability (must win over step 2).
    //
    // This is the exact ordering trap described in the phase spec. A naive fix that
    // marks columns nullable inside process_from_clause_pure (step 1) would be
    // overwritten by step 2, so this test would fail for a naive implementation.

    // SQL matching the fixture file (simplified for inference without full workspace context).
    // Use explicit AS aliases so item.alias() returns deterministic names.
    let sql = "SELECT u.user_id AS user_id, e.event_id AS event_id, e.event_type AS event_type \
               FROM u LEFT JOIN e ON u.user_id = e.user_id";

    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt");

    let mut ctx = TypeContext::new();

    // Left side: `u` has user_id (nullable: true — typical user table)
    ctx.add_model_column(
        "u",
        "user_id",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: true,
        },
    );
    ctx.add_alias("u", "u");

    // Right side: `e` has event_id declared nullable: false (source declaration).
    // Simulate the overwrite trap: first seed nullable:true (as build_type_context does),
    // then overwrite with nullable:false (as add_source_info_to_type_context does).
    ctx.add_model_column(
        "e",
        "event_id",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: true,
        },
    );
    ctx.add_model_column(
        "e",
        "event_type",
        TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        },
    );
    ctx.add_alias("e", "e");
    // Simulate add_source_info_to_type_context overwriting with the declared value:
    ctx.add_model_column(
        "e",
        "event_id",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        },
    );
    ctx.add_model_column(
        "e",
        "event_type",
        TypedColumn {
            data_type: DataType::Text,
            nullable: false,
        },
    );

    // At this point, without the pass, event_id would be nullable: false.
    // Verify the trap exists (pre-condition of the test):
    let pre_fix = ctx.lookup_column(Some("e"), "event_id");
    assert!(
        pre_fix.map(|c| !c.nullable).unwrap_or(false),
        "Pre-condition: before apply_outer_join_nullability, event_id should be non-nullable \
         (simulating the overwrite trap). If this fails, the test setup is wrong."
    );

    // Apply the outer-join nullability pass — must fire AFTER source info is seeded.
    apply_outer_join_nullability(&select_stmt, &mut ctx);

    // Infer output schema.
    let col_types = infer_select_column_types(&select_stmt, &ctx);
    let select_list = select_stmt.select_list().expect("no select list");
    let items: Vec<_> = select_list.items().collect();
    let inferred: Vec<(String, TypedColumn)> = items
        .iter()
        .zip(col_types.iter())
        .map(|(item, tc)| {
            let alias = item.alias().unwrap_or_else(|| "?".to_string());
            (alias, tc.clone())
        })
        .collect();

    // The event_id from the right side of a LEFT JOIN must be nullable.
    let event_id = inferred
        .iter()
        .find(|(a, _)| a == "event_id")
        .expect("event_id not found in inferred output");

    assert!(
        event_id.1.nullable,
        "event_id is declared nullable: false in the source, but appears on the right side of a \
         LEFT JOIN — apply_outer_join_nullability must mark it nullable: true. \
         Got nullable: {}",
        event_id.1.nullable
    );

    // user_id (left side) must stay non-nullable as-is (it's nullable:true in our sim, but we're
    // checking that the pass doesn't accidentally mark left-side columns nullable for a LEFT JOIN).
    // Actually left side was nullable:true; this just confirms no regression on the left side.
    let user_id = inferred
        .iter()
        .find(|(a, _)| a == "user_id")
        .expect("user_id not found in inferred output");
    // user_id was already nullable:true; the pass does not change it (LEFT JOIN only marks RIGHT).
    assert!(
        user_id.1.nullable,
        "user_id should remain nullable (it was declared nullable:true), got: {}",
        user_id.1.nullable
    );

    // ── Part 2: DuckDB value check ───────────────────────────────────────────
    //
    // Verify DuckDB semantics agree: a LEFT JOIN where the right side has no match
    // produces NULL for event_id, confirming the nullable inference is correct.

    let oracle = DuckDbOracle::new();
    oracle
        .execute_ddl(
            "CREATE TABLE users (user_id INTEGER NOT NULL);\
             INSERT INTO users VALUES (1), (2);\
             CREATE TABLE events (user_id INTEGER, event_id INTEGER NOT NULL, event_type VARCHAR NOT NULL);\
             INSERT INTO events VALUES (1, 100, 'click')"
        )
        .expect("setup DDL should succeed");

    // User 2 has no events → LEFT JOIN produces NULL for event_id and event_type for user 2.
    let nulls = oracle
        .count_nulls_per_column(
            "SELECT u.user_id, e.event_id, e.event_type \
             FROM users u LEFT JOIN events e ON u.user_id = e.user_id",
        )
        .expect("query should succeed");

    let event_id_nulls = nulls
        .iter()
        .find(|(name, _)| name == "event_id")
        .map(|(_, count)| *count)
        .unwrap_or(0);

    assert!(
        event_id_nulls > 0,
        "DuckDB value check: event_id (declared NOT NULL on events table) must contain NULLs \
         after a LEFT JOIN where some rows have no match. Got {} NULLs — this confirms the \
         nullable inference is semantically correct.",
        event_id_nulls
    );
}

// ---- Smoke: set-operation nullability ----

/// Smoke: `non_nullable UNION ALL nullable` infers nullable; DuckDB confirms NULLs present.
///
/// Verifies the set-op rule (§11): output column nullability is the OR of all branch
/// nullabilities. A non-nullable literal in branch 1 combined with a NULL-typed expression
/// in branch 2 must yield nullable output.
#[test]
fn smoke_union_mixed_nullability() {
    // ── Part 1: inference check — smelt must say "guard" is nullable ──────────
    let mixed_sql = build_setop_mixed_nullability_cte();

    let parse = smelt_parser::parse(&mixed_sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt");

    // Minimal context — no source columns (both branches use literals only).
    let ctx = TypeContext::new();
    let col_types = infer_select_column_types(&select_stmt, &ctx);
    let select_list = select_stmt.select_list().expect("no select list");
    let items: Vec<_> = select_list.items().collect();
    let inferred: Vec<(String, TypedColumn)> = items
        .iter()
        .zip(col_types.iter())
        .map(|(item, tc)| {
            let alias = item.alias().unwrap_or_else(|| "?".to_string());
            (alias, tc.clone())
        })
        .collect();

    let guard = inferred
        .iter()
        .find(|(a, _)| a == "guard")
        .expect("guard column not found in inferred output");
    assert!(
        guard.1.nullable,
        "UNION ALL of non-nullable (42) and nullable (NULL) must infer nullable for 'guard'. \
         Got nullable: {}",
        guard.1.nullable
    );

    // ── Part 2: DuckDB value check — confirm NULLs actually appear ───────────
    let oracle = DuckDbOracle::new();
    let nulls = oracle
        .count_nulls_per_column(&mixed_sql)
        .expect("UNION ALL query should succeed");
    let guard_nulls = nulls
        .iter()
        .find(|(name, _)| name == "guard")
        .map(|(_, count)| *count)
        .unwrap_or(0);
    assert!(
        guard_nulls > 0,
        "UNION ALL with NULL branch must produce actual NULLs for 'guard' in DuckDB. \
         Got {} NULLs — this confirms the set-op rule is needed.",
        guard_nulls
    );

    // ── Part 3: uniform non-nullable branches must infer non-nullable ─────────
    let uniform_sql = build_setop_uniform_nonnullable_cte();
    let parse2 = smelt_parser::parse(&uniform_sql);
    let root2 = parse2.syntax();
    let file2 = File::cast(root2).expect("failed to cast to File");
    let select_stmt2 = file2.select_stmt().expect("no SelectStmt");
    let ctx2 = TypeContext::new();
    let col_types2 = infer_select_column_types(&select_stmt2, &ctx2);
    let select_list2 = select_stmt2.select_list().expect("no select list");
    let items2: Vec<_> = select_list2.items().collect();
    let inferred2: Vec<(String, TypedColumn)> = items2
        .iter()
        .zip(col_types2.iter())
        .map(|(item, tc)| {
            let alias = item.alias().unwrap_or_else(|| "?".to_string());
            (alias, tc.clone())
        })
        .collect();

    let guard2 = inferred2
        .iter()
        .find(|(a, _)| a == "guard")
        .expect("guard column not found in uniform inferred output");
    assert!(
        !guard2.1.nullable,
        "UNION ALL of non-nullable (42) and non-nullable (42) must infer non-nullable for 'guard'. \
         Got nullable: {}",
        guard2.1.nullable
    );

    // DuckDB confirm: no NULLs in guard for uniform case.
    let oracle2 = DuckDbOracle::new();
    let nulls2 = oracle2
        .count_nulls_per_column(&uniform_sql)
        .expect("uniform UNION ALL query should succeed");
    let guard_nulls2 = nulls2
        .iter()
        .find(|(name, _)| name == "guard")
        .map(|(_, count)| *count)
        .unwrap_or(0);
    assert_eq!(
        guard_nulls2, 0,
        "UNION ALL of 42 UNION ALL 42 must produce zero NULLs in 'guard'. Got {}.",
        guard_nulls2
    );
}

// ---- Phase 4 regression tests: spec §11 closed-list audit ----

/// Regression: `CASE WHEN p THEN non_nullable_expr END` (no ELSE) must infer nullable.
///
/// Without an ELSE clause, the implicit default is NULL when no branch matches.
/// Spec §11: CASE without ELSE is always nullable.
#[test]
fn regression_case_without_else_nullable() {
    use smelt_db::type_inference::{infer_select_column_types, TypeContext};
    use smelt_parser::ast::File;

    // non_nullable_expr = literal 42 (nullable: false per literal inference)
    // CASE WHEN TRUE THEN 42 END — no ELSE. Must be nullable.
    let sql = "WITH data AS (SELECT 1 AS x) SELECT CASE WHEN TRUE THEN 42 END AS result FROM data";

    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt");

    let ctx = TypeContext::new();
    let col_types = infer_select_column_types(&select_stmt, &ctx);
    assert_eq!(col_types.len(), 1, "expected 1 output column");
    assert!(
        col_types[0].nullable,
        "CASE WHEN ... THEN non_nullable END (no ELSE) must infer nullable \
         — implicit default is NULL when no branch matches. Got nullable: false"
    );

    // DuckDB value check: a CASE with no matching branch returns NULL.
    let oracle = DuckDbOracle::new();
    let nulls = oracle
        .count_nulls_per_column(
            "SELECT CASE WHEN FALSE THEN 42 END AS result FROM (VALUES (1)) t(x)",
        )
        .expect("query should succeed");
    let result_nulls = nulls
        .iter()
        .find(|(name, _)| name == "result")
        .map(|(_, c)| *c)
        .unwrap_or(0);
    assert!(
        result_nulls > 0,
        "CASE with no matching branch must produce NULLs in DuckDB. \
         Got {} NULLs — confirms CASE without ELSE can be NULL.",
        result_nulls
    );
}

/// Regression: `TRY_CAST(non_nullable AS T)` must infer nullable.
///
/// TRY_CAST returns NULL when the cast fails (e.g. TRY_CAST('abc' AS INTEGER) = NULL).
/// Spec §11: TRY_CAST is in the "always nullable" list.
///
/// In smelt, TRY_CAST is not a CAST_EXPR (parser doesn't know about TRY_CAST) — it is
/// parsed as a function call with name `TRY_CAST`. Since it's not in SqlFunction, it
/// falls through to returning None/Unknown, which defaults to nullable: true.
#[test]
fn regression_try_cast_nullable() {
    use smelt_db::type_inference::{infer_select_column_types, TypeContext};
    use smelt_parser::ast::File;
    use smelt_types::TypedColumn;

    // Use a context with a non-nullable column to test TRY_CAST on it.
    let sql = "WITH data AS (SELECT CAST(42 AS INTEGER) AS nn_col) \
               SELECT TRY_CAST(nn_col AS VARCHAR) AS result FROM data";

    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt");

    let mut ctx = TypeContext::new();
    // nn_col is non-nullable
    ctx.add_cte_column(
        "data",
        "nn_col",
        TypedColumn {
            data_type: smelt_types::DataType::Integer,
            nullable: false,
        },
    );

    let col_types = infer_select_column_types(&select_stmt, &ctx);
    assert_eq!(col_types.len(), 1, "expected 1 output column");
    assert!(
        col_types[0].nullable,
        "TRY_CAST(non_nullable AS T) must infer nullable — TRY_CAST returns NULL on failure. \
         Got nullable: false"
    );

    // DuckDB value check: TRY_CAST('abc' AS INTEGER) returns NULL.
    let oracle = DuckDbOracle::new();
    let nulls = oracle
        .count_nulls_per_column(
            "SELECT TRY_CAST('abc' AS INTEGER) AS result FROM (VALUES (1)) t(x)",
        )
        .expect("query should succeed");
    let result_nulls = nulls
        .iter()
        .find(|(name, _)| name == "result")
        .map(|(_, c)| *c)
        .unwrap_or(0);
    assert!(
        result_nulls > 0,
        "TRY_CAST('abc' AS INTEGER) must return NULL in DuckDB. \
         Got {} NULLs — confirms TRY_CAST is nullable.",
        result_nulls
    );
}

/// Regression: `NULLIF(non_nullable, x)` must infer nullable.
///
/// NULLIF(a, b) returns NULL when a = b, so it can always be NULL regardless of input nullability.
/// Spec §11: NULLIF is in the "always nullable" list.
#[test]
fn regression_nullif_nullable() {
    use smelt_db::type_inference::{infer_select_column_types, TypeContext};
    use smelt_parser::ast::File;
    use smelt_types::TypedColumn;

    let sql = "WITH data AS (SELECT CAST(42 AS INTEGER) AS nn_col) \
               SELECT NULLIF(nn_col, 0) AS result FROM data";

    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt");

    let mut ctx = TypeContext::new();
    ctx.add_cte_column(
        "data",
        "nn_col",
        TypedColumn {
            data_type: smelt_types::DataType::Integer,
            nullable: false,
        },
    );

    let col_types = infer_select_column_types(&select_stmt, &ctx);
    assert_eq!(col_types.len(), 1, "expected 1 output column");
    assert!(
        col_types[0].nullable,
        "NULLIF(non_nullable, x) must infer nullable — NULLIF returns NULL when args are equal. \
         Got nullable: false"
    );

    // DuckDB value check: NULLIF(42, 42) = NULL.
    let oracle = DuckDbOracle::new();
    let nulls = oracle
        .count_nulls_per_column("SELECT NULLIF(42, 42) AS result FROM (VALUES (1)) t(x)")
        .expect("query should succeed");
    let result_nulls = nulls
        .iter()
        .find(|(name, _)| name == "result")
        .map(|(_, c)| *c)
        .unwrap_or(0);
    assert!(
        result_nulls > 0,
        "NULLIF(42, 42) must return NULL in DuckDB. \
         Got {} NULLs — confirms NULLIF is nullable.",
        result_nulls
    );
}

/// Regression: `LAG(non_nullable) OVER (...)` (no default) must infer nullable.
///
/// LAG without an explicit default returns NULL for the first row (no previous row exists).
/// Spec §11: LAG/LEAD without an explicit default is in the "always nullable" list.
#[test]
fn regression_lag_without_default_nullable() {
    use smelt_db::type_inference::{infer_select_column_types, TypeContext};
    use smelt_parser::ast::File;
    use smelt_types::TypedColumn;

    let sql = "WITH data AS (SELECT CAST(42 AS INTEGER) AS nn_col) \
               SELECT LAG(nn_col) OVER () AS result FROM data";

    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt");

    let mut ctx = TypeContext::new();
    ctx.add_cte_column(
        "data",
        "nn_col",
        TypedColumn {
            data_type: smelt_types::DataType::Integer,
            nullable: false,
        },
    );

    let col_types = infer_select_column_types(&select_stmt, &ctx);
    assert_eq!(col_types.len(), 1, "expected 1 output column");
    assert!(
        col_types[0].nullable,
        "LAG(non_nullable) OVER (...) (no default) must infer nullable — \
         LAG returns NULL for the first row. Got nullable: false"
    );

    // DuckDB value check: LAG on a single row returns NULL.
    let oracle = DuckDbOracle::new();
    let nulls = oracle
        .count_nulls_per_column("SELECT LAG(x) OVER () AS result FROM (VALUES (42)) t(x)")
        .expect("query should succeed");
    let result_nulls = nulls
        .iter()
        .find(|(name, _)| name == "result")
        .map(|(_, c)| *c)
        .unwrap_or(0);
    assert!(
        result_nulls > 0,
        "LAG(x) OVER () on a single-row table must return NULL in DuckDB. \
         Got {} NULLs — confirms LAG without default is nullable.",
        result_nulls
    );
}

/// Regression: array containment operators `@>` and `<@` with a NULL operand must infer nullable.
///
/// `@>` and `<@` are NULL-propagating: when either operand is NULL, the result is NULL.
/// Spec §11: only non-NULL-propagating operators may claim non-nullable when both operands
/// are non-nullable; `@>` / `<@` must always produce `nullable: true` since NULL propagates.
#[test]
fn regression_json_containment_operators_nullable() {
    use smelt_db::type_inference::{infer_select_column_types, TypeContext};
    use smelt_parser::ast::File;
    use smelt_types::TypedColumn;

    // Use a non-nullable integer column as operand to test that @> still claims nullable.
    let sql = "WITH data AS (SELECT ARRAY[1, 2, 3] AS nn_arr) \
               SELECT nn_arr @> ARRAY[1] AS result FROM data";

    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt");

    let mut ctx = TypeContext::new();
    ctx.add_cte_column(
        "data",
        "nn_arr",
        TypedColumn {
            data_type: smelt_types::DataType::Array(Box::new(smelt_types::DataType::Integer)),
            nullable: false,
        },
    );

    let col_types = infer_select_column_types(&select_stmt, &ctx);
    assert_eq!(col_types.len(), 1, "expected 1 output column");
    assert!(
        col_types[0].nullable,
        "@> operator must infer nullable — it is NULL-propagating (NULL @> x = NULL). \
         Got nullable: false"
    );

    // DuckDB value check: NULL @> ARRAY[1] returns NULL.
    let oracle = DuckDbOracle::new();
    let nulls = oracle
        .count_nulls_per_column(
            "SELECT (NULL::INTEGER[]) @> ARRAY[1] AS result FROM (VALUES (1)) t(x)",
        )
        .expect("query should succeed");
    let result_nulls = nulls
        .iter()
        .find(|(name, _)| name == "result")
        .map(|(_, c)| *c)
        .unwrap_or(0);
    assert!(
        result_nulls > 0,
        "NULL @> ARRAY[1] must return NULL in DuckDB. Got {} NULLs — confirms @> is nullable.",
        result_nulls
    );
}

/// Regression: `json_contains(NULL, ...)` must infer nullable.
///
/// `json_contains` is a NULL-propagating function: when either argument is NULL, the result is NULL.
/// Spec §11: only COUNT(*)/COUNT(expr) and EXISTS may claim non-nullable among aggregate/scalar funcs;
/// `json_contains` is a regular scalar function that propagates NULLs.
#[test]
fn regression_json_contains_nullable() {
    use smelt_db::type_inference::{infer_select_column_types, TypeContext};
    use smelt_parser::ast::File;
    use smelt_types::TypedColumn;

    // Use a non-nullable column to confirm that even with non-nullable input,
    // json_contains must claim nullable (it propagates NULLs).
    let sql = "WITH data AS (SELECT CAST('{\"a\":1}' AS VARCHAR) AS nn_json) \
               SELECT json_contains(nn_json, '{\"a\":1}') AS result FROM data";

    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt");

    let mut ctx = TypeContext::new();
    ctx.add_cte_column(
        "data",
        "nn_json",
        TypedColumn {
            data_type: smelt_types::DataType::Text,
            nullable: false,
        },
    );

    let col_types = infer_select_column_types(&select_stmt, &ctx);
    assert_eq!(col_types.len(), 1, "expected 1 output column");
    assert!(
        col_types[0].nullable,
        "json_contains must infer nullable — it is NULL-propagating (json_contains(NULL, ...) = NULL). \
         Got nullable: false"
    );

    // DuckDB value check: json_contains(NULL, ...) returns NULL.
    let oracle = DuckDbOracle::new();
    let nulls = oracle
        .count_nulls_per_column(
            r#"SELECT json_contains(NULL::JSON, '{"a":1}'::JSON) AS result FROM (VALUES (1)) t(x)"#,
        )
        .expect("query should succeed");
    let result_nulls = nulls
        .iter()
        .find(|(name, _)| name == "result")
        .map(|(_, c)| *c)
        .unwrap_or(0);
    assert!(
        result_nulls > 0,
        "json_contains(NULL, ...) must return NULL in DuckDB. \
         Got {} NULLs — confirms json_contains is nullable.",
        result_nulls
    );
}

// ---- Property tests ----

proptest! {
    #![proptest_config(ProptestConfig::with_cases(
        std::env::var("PROPTEST_CASES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(256)
    ))]

    /// Core nullability soundness property: for every generated single-table query over
    /// generated data (nullable columns actually populated with NULLs), every column smelt
    /// infers as `nullable: false` must contain zero NULLs in DuckDB results.
    ///
    /// Runtime-erroring queries are discarded (same policy as `type_property_tests.rs`).
    ///
    /// Structural invariants checked on every non-skipped case:
    ///   1. Builder-divergence check: the CTE builder and real-table builder must emit the same
    ///      number of SELECT columns (both builders are compared to each other, independently of
    ///      smelt's inference completeness).
    ///   2. Every inferred column's alias appears in the observed result set (name-based match).
    ///   3. At least one non-nullable column was asserted on (non-vacuous gate) — relaxed when
    ///      smelt under-counts its own inferred columns (see "smelt under-count" comment below).
    #[test]
    fn prop_nullability_sound(
        (columns, shape, expr_kinds, func_indices) in test_scenario_strategy()
    ) {
        // Generate expressions
        let mut exprs = Vec::new();
        for (i, kind) in expr_kinds.iter().enumerate() {
            let func_idx = func_indices.get(i).copied().unwrap_or(0);
            if let Some(expr) = generate_expr(&columns, *kind, i, func_idx) {
                exprs.push(expr);
            }
        }

        prop_assume!(!exprs.is_empty());

        // Inject guaranteed non-nullable-origin guard columns into every generated query
        // so that the non-vacuous assertion (checked_non_nullable >= 1) always has teeth.
        //
        // The challenge: `assemble_cte_query` and `build_select_query` apply shape-specific
        // filters to the expression list.  A single guard type can be filtered out:
        //
        //   - For Scalar/Window shapes: when user exprs are all aggregates, the builder keeps
        //     only aggregates, dropping any scalar literal guard.
        //   - For GROUP-BY shapes: the builder keeps only aggregates, dropping scalar literals.
        //   - For Distinct shapes: the builder drops aggregates and windows.
        //
        // Solution: inject BOTH a scalar guard (`42 AS nn_scalar_guard`) AND an aggregate
        // guard (`COUNT(*) AS nn_count_guard`).  For every shape-filter combination, at least
        // one of these survives:
        //
        //   Scalar/Window with no aggs:         both survive    → nn_scalar_guard asserted ✓
        //   Scalar/Window with aggs, no windows: agg-filter     → nn_count_guard asserted ✓
        //   Scalar/Window with aggs + windows:   drop-agg-filter → nn_scalar_guard asserted ✓
        //   Distinct:                            drop agg+window → nn_scalar_guard asserted ✓
        //   GroupBy/GroupByHaving/GroupByWindow: agg-only filter → nn_count_guard asserted ✓
        //
        // Smelt infers `42` as `nullable: false` (numeric literal; see `literal.rs`) and
        // `COUNT(*)` as `nullable: false` (aggregate that never returns NULL).
        let is_group_by_shape = matches!(
            shape,
            QueryShape::GroupBy { .. } | QueryShape::GroupByHaving { .. } | QueryShape::GroupByWindow { .. }
        );
        // Scalar (non-aggregate) guard — survives when scalars are kept.
        // NOT injected for GROUP-BY shapes: a non-aggregated literal in a GROUP BY SELECT list
        // is invalid SQL (would need to be in GROUP BY clause or be an aggregate).
        let scalar_guard_alias = "nn_scalar_guard";
        let agg_guard_alias = "nn_count_guard";
        if !is_group_by_shape {
            exprs.push(TypedExpr {
                sql: "42".to_string(),
                alias: scalar_guard_alias.to_string(),
                expected_smelt_type: DataType::Integer,
            });
        }
        // Aggregate guard — survives when aggregates are kept (GROUP-BY and agg-Scalar shapes).
        exprs.push(TypedExpr {
            sql: "COUNT(*)".to_string(),
            alias: agg_guard_alias.to_string(),
            expected_smelt_type: DataType::BigInt,
        });

        // Build the CTE-style SQL (for smelt inference — uses literals, not actual rows)
        let cte_sql = assemble_cte_query(&columns, &exprs, &shape);

        // Count how many SELECT columns the CTE builder actually emitted by running the CTE
        // SQL through DuckDB.  This is the authoritative builder column count — independent of
        // smelt's inference completeness (the smelt parser may under-count when it cannot parse
        // certain expressions, e.g. JSON -> operator).  We need a separate DuckDB connection so
        // the CTE query runs in a fresh in-memory DB without any real table.
        let cte_oracle = DuckDbOracle::new();
        let cte_builder_col_count = match cte_oracle.count_nulls_per_column(&cte_sql) {
            Ok(cols) => cols.len(),
            Err(_) => return Ok(()), // skip if CTE SQL is invalid (shouldn't normally happen)
        };
        if cte_builder_col_count == 0 {
            return Ok(()); // skip if CTE produces no columns
        }

        // Run smelt inference on the CTE query
        let inferred = run_smelt_inference_with_nullability(&cte_sql, &columns);
        if inferred.is_empty() {
            return Ok(()); // skip if inference fails
        }

        // Build the real-data version: CREATE TABLE + INSERT rows with NULLs + SELECT
        let oracle = DuckDbOracle::new();
        let (setup_sql, select_sql) = match build_null_bearing_query(&columns, &exprs, &shape) {
            Some(q) => q,
            None => return Ok(()), // skip if we can't generate data for this shape
        };

        // Execute setup
        if oracle.execute_ddl(&setup_sql).is_err() {
            return Ok(()); // skip if DDL fails (e.g. unsupported types)
        }

        // Run the value-based check
        let observed_nulls = match oracle.count_nulls_per_column(&select_sql) {
            Ok(n) => n,
            Err(_) => return Ok(()), // runtime error — discard
        };

        // ── Invariant 1: Builder-divergence check ───────────────────────────────
        //
        // The CTE builder (assemble_cte_query) and the real-table builder
        // (build_select_query) must emit the same number of SELECT columns.
        // We compare the *two builders* against each other — NOT smelt's inferred
        // column count — so this check is independent of smelt's inference completeness.
        //
        // A mismatch here is a genuine harness bug: the two builders diverged in their
        // SELECT list construction, meaning the oracle is comparing incompatible query shapes.
        prop_assert!(
            cte_builder_col_count == observed_nulls.len(),
            "Harness builder-divergence violation: CTE builder emitted {} columns but DuckDB \
             (real-table builder) returned {} columns.\n\
             The two builders produced different SELECT lists — fix the harness.\n\
             CTE column count (parsed from CTE SQL): {}\n\
             Observed names: {:?}\n\
             CTE SQL: {}\n\
             real-data SELECT SQL: {}",
            cte_builder_col_count, observed_nulls.len(),
            cte_builder_col_count,
            observed_nulls.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            cte_sql, select_sql
        );

        // ── Smelt under-count detection ──────────────────────────────────────────
        //
        // smelt's `infer_select_column_types` may return fewer columns than the query
        // actually has (an inference-completeness gap, not a nullability-soundness issue).
        // When smelt under-counts, it made NO claim about the missing columns — there is
        // nothing to verify for those columns.  Skipping them is sound under the one-sided
        // contract: `nullable: false` is a hard guarantee, but only for columns smelt
        // actually inferred.  A column smelt did not infer cannot have been falsely claimed
        // as non-nullable.
        //
        // IMPORTANT: this is NOT the same as silently skipping a potential violation.
        // The columns smelt DID infer are still fully soundness-checked by name in the loop
        // below (invariant 2).  Only the *non-vacuous coverage requirement* is relaxed when
        // smelt under-counted, because the injected guards may be among the columns smelt
        // dropped — and that gap is smelt's inference incompleteness, not a soundness defect.
        let smelt_undercounted = inferred.len() < cte_builder_col_count;

        // ── Invariant 2: Name-based soundness check ──────────────────────────────
        //
        // For every column smelt inferred, look it up by alias in the observed result set
        // and assert: (a) it exists (alias matches a real column — catches builder alias
        // divergence), (b) if smelt claims nullable: false, DuckDB observed zero NULLs.
        //
        // This loop runs regardless of whether smelt under-counted.  It is the core
        // soundness assertion: every `nullable: false` claim smelt made must hold in data.
        let mut checked_non_nullable: usize = 0;
        for (alias, tc) in &inferred {
            // Find the observed column by name (DuckDB returns the AS alias as the column name).
            let obs_entry = observed_nulls.iter().find(|(name, _)| name == alias);

            // If an inferred column's alias is not found among observed names, that is a
            // harness bug: the two builders disagree on column aliases.  Fail loudly rather
            // than defaulting to 0 (which would silently pass a potentially real violation).
            prop_assert!(
                obs_entry.is_some(),
                "Harness alias mismatch: inferred column '{}' not found among observed column names {:?}.\n\
                 The CTE builder and real-table builder produced different aliases — fix the harness.\n\
                 CTE SQL: {}\n\
                 real-data SELECT SQL: {}",
                alias,
                observed_nulls.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
                cte_sql, select_sql
            );

            if !tc.nullable {
                let obs_null_count = obs_entry.map(|(_, c)| *c).unwrap_or(0);
                checked_non_nullable += 1;

                prop_assert!(
                    obs_null_count == 0,
                    "Nullability soundness violation: column '{}'\n\
                     smelt inferred nullable: false, but DuckDB returned {} NULLs\n\
                     CTE SQL: {}\n\
                     real-data SELECT SQL: {}",
                    alias, obs_null_count,
                    cte_sql, select_sql
                );
            }
        }

        // ── Invariant 3: Non-vacuous gate ────────────────────────────────────────
        //
        // We always inject guard expression(s) that smelt must infer as `nullable: false`:
        //   - Non-GROUP-BY shapes: both `42 AS nn_scalar_guard` (literal) and
        //     `COUNT(*) AS nn_count_guard` (aggregate).  At least one survives every filter.
        //   - GROUP-BY shapes: only `COUNT(*) AS nn_count_guard` (aggregate); literals are
        //     invalid SQL in GROUP BY select lists without being in the GROUP BY clause.
        //
        // When smelt under-counted its own output columns (smelt_undercounted == true), the
        // injected guards may be among the columns smelt failed to infer — so the non-vacuous
        // gate is NOT enforced.  This is safe: the smelt defect (under-counting) is an
        // inference-completeness gap, not a soundness gap.  We contribute no coverage for this
        // case rather than falsely failing a test that smelt cannot satisfy yet.
        //
        // When smelt inferred the full column set, the gates MUST fire: if they don't, smelt
        // regressed on non-nullable literal/COUNT(*) inference, or both guards were filtered
        // out by the builder (harness bug).
        if !smelt_undercounted {
            prop_assert!(
                checked_non_nullable >= 1,
                "Vacuous coverage: no non-nullable column was asserted on in this case.\n\
                 Guards injected: scalar='{}' (skipped for group-by: {}), aggregate='{}'\n\
                 All inferred columns are nullable — either smelt regressed on non-nullable\n\
                 literals/COUNT(*) inference, both guards were filtered out, or the harness has a bug.\n\
                 Inferred columns: {:?}\n\
                 CTE SQL: {}",
                scalar_guard_alias,
                is_group_by_shape,
                agg_guard_alias,
                inferred.iter().map(|(a, tc)| format!("{}:nullable={}", a, tc.nullable)).collect::<Vec<_>>(),
                cte_sql
            );
        }
    }

    /// Nullability soundness property for two-table joins (INNER / LEFT / RIGHT / FULL).
    ///
    /// For each generated case:
    ///   1. Build a CTE query with `l` and `r` aliases using disjoint keys (l.lkey=1, r.rkey=999).
    ///      The SELECT list includes a guard column from each side: `l.l_guard` and `r.r_guard`,
    ///      both sourced from the literal `42` (non-nullable in each CTE's definition).
    ///   2. Apply `apply_outer_join_nullability` so LEFT JOIN marks r.* nullable,
    ///      RIGHT JOIN marks l.* nullable, FULL JOIN marks both.
    ///   3. Run smelt inference on the CTE query.
    ///   4. Build real tables (LEFT_TABLE, RIGHT_TABLE) with disjoint join keys → outer joins
    ///      produce actual NULLs in null-supplying side columns.
    ///   5. For every column smelt infers as `nullable: false`, assert null_count == 0 in DuckDB.
    ///
    /// Soundness check: if smelt over-claims non-nullable (e.g. keeps r.r_guard non-nullable
    /// in a LEFT JOIN), DuckDB will observe NULLs there (outer join propagation), and the
    /// test fails — catching a regression in `apply_outer_join_nullability`.
    ///
    /// Non-vacuous gate: at least one non-nullable column must be asserted. The l_guard
    /// (a literal 42 from the preserved side) satisfies this for all outer join types except
    /// FULL JOIN (where both sides may be nullable). For FULL JOIN with 0 rows in INNER
    /// projection, DuckDB returns 0 rows — all null_counts are 0, making the soundness check
    /// trivially pass (no NULLs observed). The gate is relaxed for FULL JOIN.
    #[test]
    fn prop_nullability_sound_joins(
        (left_cols, right_cols) in (column_pool_strategy(), column_pool_strategy()),
        join_kind_idx in 0usize..4,
    ) {
        let join_kind = match join_kind_idx {
            0 => JoinKind::Inner,
            1 => JoinKind::Left,
            2 => JoinKind::Right,
            _ => JoinKind::Full,
        };

        // Build the CTE query for smelt inference.
        let cte_sql = build_join_cte_query(&left_cols, &right_cols, join_kind);

        // Build the TypeContext: left and right CTE columns (all source columns nullable),
        // then inject guard columns as non-nullable (they are defined as literal 42).
        let inferred = run_join_smelt_inference(&cte_sql, &left_cols, &right_cols, join_kind);
        if inferred.is_empty() {
            return Ok(()); // skip if inference fails (parse error etc.)
        }

        // Build real DuckDB tables and the join SELECT.
        let (setup_sql, select_sql) = build_join_real_table_query(&left_cols, &right_cols, join_kind);
        let oracle = DuckDbOracle::new();
        if oracle.execute_ddl(&setup_sql).is_err() {
            return Ok(()); // skip if DDL fails (unsupported types)
        }

        // Run the join query and collect null counts.
        let observed_nulls = match oracle.count_nulls_per_column(&select_sql) {
            Ok(n) => n,
            Err(_) => return Ok(()), // runtime error — discard
        };

        // Soundness check: for every column smelt claims non-nullable, DuckDB must show 0 NULLs.
        let mut checked_non_nullable: usize = 0;
        for (alias, tc) in &inferred {
            if !tc.nullable {
                let obs_null_count = observed_nulls
                    .iter()
                    .find(|(name, _)| name == alias)
                    .map(|(_, c)| *c)
                    .unwrap_or(0);
                checked_non_nullable += 1;
                prop_assert!(
                    obs_null_count == 0,
                    "Join nullability soundness violation: column '{}'\n\
                     join_kind={:?}, smelt inferred nullable: false, \
                     but DuckDB returned {} NULLs\n\
                     CTE SQL: {}\n\
                     DuckDB SELECT SQL: {}",
                    alias, join_kind, obs_null_count,
                    cte_sql, select_sql
                );
            }
        }

        // Non-vacuous gate: for INNER and LEFT/RIGHT (where preserved side has non-nullable guard),
        // at least one column must have been asserted. Relaxed for FULL JOIN where both sides
        // may be nullable (0 non-nullable columns is valid for FULL JOIN).
        // Also skip if smelt under-counted columns (inference completeness gap).
        let smelt_col_count = inferred.len();
        let builder_col_count = left_cols.len() + right_cols.len() + 2; // +2 for l_guard, r_guard
        let smelt_undercounted = smelt_col_count < builder_col_count;
        if !smelt_undercounted && !matches!(join_kind, JoinKind::Full) {
            prop_assert!(
                checked_non_nullable >= 1,
                "Join vacuous coverage: no non-nullable column was asserted.\n\
                 join_kind={:?}, inferred columns: {:?}\n\
                 CTE SQL: {}",
                join_kind,
                inferred.iter().map(|(a, tc)| format!("{}:nullable={}", a, tc.nullable)).collect::<Vec<_>>(),
                cte_sql
            );
        }
    }

    /// Nullability soundness property for set operations (UNION ALL / UNION / INTERSECT / EXCEPT).
    ///
    /// For each generated case:
    ///   1. Generate a column pool (used for both branches — same column types).
    ///   2. Build a query:
    ///      ```sql
    ///      WITH data AS (SELECT col_casts...)
    ///      SELECT col1, ..., 42 AS guard FROM data
    ///      [UNION ALL | UNION | INTERSECT | EXCEPT]
    ///      SELECT col1, ..., [42 | CAST(NULL AS INTEGER)] AS guard FROM data
    ///      ```
    ///      In the "mixed" variant, branch 2 has a NULL guard → output guard is nullable.
    ///      In the "uniform" variant, both branches have 42 → output guard is non-nullable.
    ///   3. Run smelt inference to get column nullabilities.
    ///   4. Run DuckDB on the same query (using CTEs so no real tables are needed).
    ///   5. For every column smelt claims non-nullable, assert null_count == 0 in DuckDB.
    ///
    /// This property verifies that `promote_types` (used inside `infer_select_column_types`
    /// for set operations) correctly applies the OR rule: output is nullable iff any branch is.
    #[test]
    fn prop_nullability_sound_setops(
        columns in column_pool_strategy(),
        setop_idx in 0usize..4,
        mixed_guard in proptest::bool::ANY,
    ) {
        prop_assume!(!columns.is_empty());

        // Pick set operation kind: 0=UNION ALL, 1=UNION, 2=INTERSECT, 3=EXCEPT
        let setop_keyword = match setop_idx {
            0 => "UNION ALL",
            1 => "UNION",
            2 => "INTERSECT",
            _ => "EXCEPT",
        };

        // Build CTE data definition.
        let cte_data_items: Vec<String> = columns
            .iter()
            .map(|c| format!("{} AS {}", c.cast_sql, c.name))
            .collect();
        let col_names: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
        let col_select = col_names.join(", ");

        // Branch 2 guard: if mixed, use NULL; otherwise use 42 (non-nullable).
        let branch2_guard = if mixed_guard { "CAST(NULL AS INTEGER)" } else { "42" };

        let cte_sql = format!(
            "WITH data AS (SELECT {data_items}) \
             SELECT {col_select}, 42 AS guard FROM data \
             {setop} \
             SELECT {col_select}, {b2_guard} AS guard FROM data",
            data_items = cte_data_items.join(", "),
            setop = setop_keyword,
            b2_guard = branch2_guard
        );

        // Run smelt inference.
        let parse = smelt_parser::parse(&cte_sql);
        let root = parse.syntax();
        let file = match File::cast(root) {
            Some(f) => f,
            None => return Ok(()),
        };
        let select_stmt = match file.select_stmt() {
            Some(s) => s,
            None => return Ok(()),
        };

        // Build context: source columns are nullable (they come from the CTE).
        let mut ctx = TypeContext::new();
        for col in &columns {
            ctx.add_cte_column("data", &col.name, TypedColumn::nullable(col.data_type.clone()));
        }

        let col_types = infer_select_column_types(&select_stmt, &ctx);
        let select_list = match select_stmt.select_list() {
            Some(sl) => sl,
            None => return Ok(()),
        };
        let items: Vec<_> = select_list.items().collect();
        let inferred: Vec<(String, TypedColumn)> = items
            .iter()
            .zip(col_types.iter())
            .map(|(item, tc)| {
                let alias = item.alias().unwrap_or_else(|| "?".to_string());
                (alias, tc.clone())
            })
            .collect();

        if inferred.is_empty() {
            return Ok(());
        }

        // Run DuckDB on the same CTE SQL (no real tables needed — both branches use the CTE).
        let oracle = DuckDbOracle::new();
        let observed_nulls = match oracle.count_nulls_per_column(&cte_sql) {
            Ok(n) => n,
            Err(_) => return Ok(()), // runtime error — discard
        };

        // Soundness check.
        let mut checked_non_nullable: usize = 0;
        for (alias, tc) in &inferred {
            let obs_null_count = observed_nulls
                .iter()
                .find(|(name, _)| name == alias)
                .map(|(_, c)| *c)
                .unwrap_or(0);

            if !tc.nullable {
                checked_non_nullable += 1;
                prop_assert!(
                    obs_null_count == 0,
                    "Set-op nullability soundness violation: column '{}'\n\
                     setop={}, mixed_guard={}, smelt inferred nullable: false, \
                     but DuckDB returned {} NULLs\n\
                     SQL: {}",
                    alias, setop_keyword, mixed_guard, obs_null_count,
                    cte_sql
                );
            }
        }

        // Non-vacuous gate: when guard is non-nullable in both branches, smelt must infer
        // guard as non-nullable. Smelt over-counting (inferred.len() > observed_nulls.len())
        // is treated as harmless (no violation possible for extra inferred columns).
        // Gate only fires for the uniform (non-mixed) case where both guards are `42`.
        let smelt_col_count = inferred.len();
        let builder_col_count = observed_nulls.len();
        let smelt_undercounted = smelt_col_count < builder_col_count;
        if !mixed_guard && !smelt_undercounted {
            prop_assert!(
                checked_non_nullable >= 1,
                "Set-op vacuous coverage: no non-nullable column was asserted.\n\
                 setop={}, both branches have 42 AS guard — smelt must infer guard as non-nullable.\n\
                 Inferred columns: {:?}\n\
                 SQL: {}",
                setop_keyword,
                inferred.iter().map(|(a, tc)| format!("{}:nullable={}", a, tc.nullable)).collect::<Vec<_>>(),
                cte_sql
            );
        }
    }
}

// ---- Generator reachability smoke tests ----
//
// `prop_nullability_sound` drives `test_scenario_strategy()` / `generate_expr` —
// the exact same shared generators `type_property_tests.rs` uses (see
// `prop_helpers::generators`), which were widened in July 2026 to cover
// temporal/interval arithmetic, decimal ops and casts, EXTRACT(EPOCH), and
// mixed naive/tz-aware timestamp comparisons. Nothing in this file's use of
// those generators narrows that space back down. These are statistical
// guards, mirroring `type_property_tests.rs`'s `mod reachability`: over a
// deterministic sample of generated scenarios, assert at least one occurrence
// of each "hard" path reaches the nullability property, so a future edit that
// silently stops emitting one of these paths (here, or upstream in the shared
// generator) is caught rather than papered over by vacuous coverage.
mod reachability {
    use super::{generate_expr, test_scenario_strategy, TypedExpr};
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;

    /// Deterministically sample `n` generated expression-kind corpora from the
    /// same top-level scenario strategy `prop_nullability_sound` drives, and
    /// render each surviving expression's raw SQL text (not the assembled
    /// query) — sufficient for the substring/token reachability checks below.
    fn sample_generated_expr_sql(n: usize) -> Vec<String> {
        let mut runner = TestRunner::deterministic();
        let strat = test_scenario_strategy();
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let tree = strat
                .new_tree(&mut runner)
                .expect("strategy generated a value");
            let (columns, _shape, expr_kinds, func_indices) = tree.current();
            let mut exprs: Vec<TypedExpr> = Vec::new();
            for (i, kind) in expr_kinds.iter().enumerate() {
                let func_idx = func_indices.get(i).copied().unwrap_or(0);
                if let Some(expr) = generate_expr(&columns, *kind, i, func_idx) {
                    exprs.push(expr);
                }
            }
            out.extend(exprs.into_iter().map(|e| e.sql));
        }
        out
    }

    const N: usize = 500;

    fn has_binop(corpus: &[String], left_prefix: &str, ops: &[&str], right_prefix: &str) -> bool {
        corpus.iter().any(|sql| {
            for token_l in tokens_starting_with(sql, left_prefix) {
                for op in ops {
                    let needle_lr = format!("{token_l} {op} ");
                    if let Some(pos) = sql.find(&needle_lr) {
                        let rest = &sql[pos + needle_lr.len()..];
                        if starts_with_prefix_token(rest, right_prefix) {
                            return true;
                        }
                    }
                }
            }
            false
        })
    }

    fn tokens_starting_with(sql: &str, prefix: &str) -> Vec<String> {
        sql.split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|t| t.starts_with(prefix) && !t.is_empty())
            .map(|t| t.to_string())
            .collect()
    }

    fn starts_with_prefix_token(rest: &str, prefix: &str) -> bool {
        let tok: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        tok.starts_with(prefix) && !tok.is_empty()
    }

    #[test]
    fn reaches_interval_plus_timestamp() {
        let corpus = sample_generated_expr_sql(N);
        let hit = has_binop(&corpus, "ts_col", &["+", "-"], "interval_col")
            || has_binop(&corpus, "interval_col", &["+", "-"], "ts_col")
            || has_binop(&corpus, "tstz_col", &["+", "-"], "interval_col")
            || has_binop(&corpus, "interval_col", &["+", "-"], "tstz_col");
        assert!(
            hit,
            "nullability generators never produced interval±timestamp over {N} cases"
        );
    }

    #[test]
    fn reaches_temporal_difference() {
        let corpus = sample_generated_expr_sql(N);
        let hit = has_binop(&corpus, "ts_col", &["-"], "ts_col")
            || has_binop(&corpus, "tstz_col", &["-"], "tstz_col");
        assert!(
            hit,
            "nullability generators never produced a temporal difference over {N} cases"
        );
    }

    #[test]
    fn reaches_decimal_arithmetic() {
        let corpus = sample_generated_expr_sql(N);
        assert!(
            has_binop(&corpus, "dec_col", &["+"], "dec_col")
                || has_binop(&corpus, "dec_col", &["*"], "dec_col")
                || has_binop(&corpus, "dec_col", &["/"], "dec_col"),
            "nullability generators never produced decimal binary arithmetic over {N} cases"
        );
    }

    #[test]
    fn reaches_decimal_cast() {
        let corpus = sample_generated_expr_sql(N);
        assert!(
            corpus.iter().any(|s| s.contains("DECIMAL(12,3)")),
            "nullability generators never produced CAST(... AS DECIMAL(12,3)) over {N} cases"
        );
    }

    #[test]
    fn reaches_extract_epoch() {
        let corpus = sample_generated_expr_sql(N);
        assert!(
            corpus.iter().any(|s| s.contains("EXTRACT(EPOCH")),
            "nullability generators never produced EXTRACT(EPOCH ...) over {N} cases"
        );
    }

    #[test]
    fn reaches_mixed_tz_comparison() {
        let corpus = sample_generated_expr_sql(N);
        let cmp = &["=", "<>", "<", ">", "<=", ">="];
        let hit = has_binop(&corpus, "ts_col", cmp, "tstz_col")
            || has_binop(&corpus, "tstz_col", cmp, "ts_col");
        assert!(
            hit,
            "nullability generators never produced a mixed-tz comparison over {N} cases"
        );
    }
}

// ---- Helpers ----

/// Parse a CTE query with smelt and return `(alias, TypedColumn)` pairs with nullability.
///
/// All source columns are treated as nullable (the generator builds nullable sources).
///
/// Note: smelt's `infer_select_column_types` may return fewer columns than the query
/// actually has (an inference-completeness gap). The returned vec length may therefore
/// be less than `count_select_items_in_cte_query` on the same SQL. Callers must not
/// assume the two agree — that discrepancy is handled in the property test harness.
fn run_smelt_inference_with_nullability(
    sql: &str,
    columns: &[TypedSource],
) -> Vec<(String, TypedColumn)> {
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = match File::cast(root) {
        Some(f) => f,
        None => return vec![],
    };
    let select_stmt = match file.select_stmt() {
        Some(s) => s,
        None => return vec![],
    };

    let mut ctx = TypeContext::new();
    for col in columns {
        // All generator-defined sources are nullable (the CTE has no non-null guarantee)
        ctx.add_cte_column(
            "data",
            &col.name,
            TypedColumn::nullable(col.data_type.clone()),
        );
    }

    let column_types = infer_select_column_types(&select_stmt, &ctx);

    let select_list = match select_stmt.select_list() {
        Some(sl) => sl,
        None => return vec![],
    };
    let items: Vec<_> = select_list.items().collect();

    items
        .iter()
        .zip(column_types.iter())
        .map(|(item, tc)| {
            let alias = item.alias().unwrap_or_else(|| "?".to_string());
            (alias, tc.clone())
        })
        .collect()
}

/// Run smelt inference on a two-table JOIN query and return `(alias, TypedColumn)` pairs.
///
/// Builds the TypeContext with left and right CTE columns, injects guard columns as
/// non-nullable (they are defined as `42` in the CTE), applies
/// `apply_outer_join_nullability`, then calls `infer_select_column_types`.
///
/// Returns an empty vec on parse/inference failure (caller should skip).
fn run_join_smelt_inference(
    cte_sql: &str,
    left_cols: &[TypedSource],
    right_cols: &[TypedSource],
    join_kind: JoinKind,
) -> Vec<(String, TypedColumn)> {
    let parse = smelt_parser::parse(cte_sql);
    let root = parse.syntax();
    let file = match File::cast(root) {
        Some(f) => f,
        None => return vec![],
    };
    let select_stmt = match file.select_stmt() {
        Some(s) => s,
        None => return vec![],
    };

    let mut ctx = TypeContext::new();

    // Register left CTE columns as nullable (source data has NULLs at ~50% density).
    for col in left_cols {
        ctx.add_cte_column(
            "l",
            &format!("l_{}", col.name),
            TypedColumn::nullable(col.data_type.clone()),
        );
    }
    // Left guard: literal 42, seeded as non-nullable.
    ctx.add_cte_column(
        "l",
        "l_guard",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        },
    );
    // Left key: always non-nullable (it's a literal).
    ctx.add_cte_column(
        "l",
        "lkey",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        },
    );

    // Register right CTE columns as nullable.
    for col in right_cols {
        ctx.add_cte_column(
            "r",
            &format!("r_{}", col.name),
            TypedColumn::nullable(col.data_type.clone()),
        );
    }
    // Right guard: literal 42, seeded as non-nullable.
    ctx.add_cte_column(
        "r",
        "r_guard",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        },
    );
    // Right key: always non-nullable.
    ctx.add_cte_column(
        "r",
        "rkey",
        TypedColumn {
            data_type: DataType::Integer,
            nullable: false,
        },
    );

    // Register aliases so qualified references l.l_col → l CTE, r.r_col → r CTE.
    ctx.add_alias("l", "l");
    ctx.add_alias("r", "r");

    // Apply the outer-join nullability pass AFTER seeding the context.
    // This is the critical step: it must mark the null-supplying side's columns as nullable.
    apply_outer_join_nullability(&select_stmt, &mut ctx);

    let _ = join_kind; // already applied via apply_outer_join_nullability on the SQL

    let column_types = infer_select_column_types(&select_stmt, &ctx);
    let select_list = match select_stmt.select_list() {
        Some(sl) => sl,
        None => return vec![],
    };
    let items: Vec<_> = select_list.items().collect();

    items
        .iter()
        .zip(column_types.iter())
        .map(|(item, tc)| {
            let alias = item.alias().unwrap_or_else(|| "?".to_string());
            (alias, tc.clone())
        })
        .collect()
}
