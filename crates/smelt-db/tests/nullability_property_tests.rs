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
    assemble_cte_query, generate_expr, test_scenario_strategy, QueryShape, TypedExpr, TypedSource,
};
use prop_helpers::null_data::{
    build_null_bearing_query, check_nullability_soundness, smoke_coalesce_non_nullable_setup,
    smoke_nullable_passthrough_setup,
};

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

// ---- Property test ----

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
    ///   1. Inferred column count == observed column count (harness mirror parity).
    ///   2. Every inferred column's alias appears in the observed result set (name-based match).
    ///   3. At least one non-nullable column was asserted on (non-vacuous gate).
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

        // Structural mirror invariant: inferred column count must equal observed column count.
        // A mismatch means the two builders (assemble_cte_query / build_select_query) diverged
        // in their SELECT list construction — that is itself a harness bug that must be fixed,
        // not silently ignored.
        prop_assert!(
            inferred.len() == observed_nulls.len(),
            "Harness mirror parity violation: smelt inferred {} columns but DuckDB returned {} columns.\n\
             The CTE builder and real-table builder produced different SELECT lists — fix the harness.\n\
             Inferred aliases: {:?}\n\
             Observed names: {:?}\n\
             CTE SQL: {}\n\
             real-data SELECT SQL: {}",
            inferred.len(), observed_nulls.len(),
            inferred.iter().map(|(a, _)| a.as_str()).collect::<Vec<_>>(),
            observed_nulls.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            cte_sql, select_sql
        );

        // Name-based matching: look up each inferred column's alias in the observed result set.
        // This is robust to any residual ordering differences and makes the alignment transparent.
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

        // Non-vacuous gate: every case must have exercised at least one non-nullable column.
        //
        // We always inject guard expression(s) that smelt must infer as `nullable: false`:
        //   - Non-GROUP-BY shapes: both `42 AS nn_scalar_guard` (literal) and
        //     `COUNT(*) AS nn_count_guard` (aggregate).  At least one survives every filter.
        //   - GROUP-BY shapes: only `COUNT(*) AS nn_count_guard` (aggregate); literals are
        //     invalid SQL in GROUP BY select lists without being in the GROUP BY clause.
        //
        // If this assertion fires it means smelt regressed on inferring a known non-nullable
        // origin as nullable, OR both guards were filtered out by a builder (harness bug).
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

// ---- Helpers ----

/// Parse a CTE query with smelt and return `(alias, TypedColumn)` pairs with nullability.
///
/// All source columns are treated as nullable (the generator builds nullable sources).
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
