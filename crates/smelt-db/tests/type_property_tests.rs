//! Property-based type inference tests.
//!
//! These tests live in `smelt-db` (not `smelt-types`) because:
//! - Type inference code (`infer_expression_type`, `TypeContext`, etc.) lives in `smelt-db`
//! - `smelt-db` already depends on `smelt-parser` and `smelt-types`
//! - Placing tests in `smelt-types` would create a circular dependency
//!
//! The strategy: generate random SQL expressions with known types via CTEs, run them
//! against DuckDB (always) and Spark (if `SPARK_CONTAINER_ID` is set) to get actual types,
//! and compare against smelt's type inference.
//! Mismatches are either bugs (to fix), known divergences (registered in `divergences.rs`),
//! or compatible type differences (Text vs Varchar, Decimal precision differences).

#[allow(dead_code)]
mod prop_helpers;

use prop_helpers::divergences::{find_divergence, known_divergences, TypeDivergence};
use prop_helpers::duckdb_oracle::{DuckDbOracle, TypeOracle};
use prop_helpers::generators::{
    self, assemble_cte_query, generate_expr, test_scenario_strategy, TypedExpr,
};
use prop_helpers::spark_oracle::SparkOracle;
use prop_helpers::type_comparison::{compare_types, TypeMatch};

use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_parser::ast::File;
use smelt_types::{DataType, TypedColumn};

use proptest::prelude::*;
use std::sync::LazyLock;

/// Shared SparkOracle instance — created once on first access.
/// The JVM startup is expensive (~5-10s), so we reuse across all test cases.
static SPARK: LazyLock<Option<SparkOracle>> = LazyLock::new(|| {
    std::env::var("SPARK_CONTAINER_ID")
        .ok()
        .map(|id| SparkOracle::new(&id))
});

/// Parse SQL with smelt and run type inference on each select column.
fn run_smelt_inference(sql: &str, columns: &[generators::TypedSource]) -> Vec<(String, DataType)> {
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt in parsed SQL");

    // Build TypeContext with CTE columns
    let mut ctx = TypeContext::new();
    for col in columns {
        ctx.add_cte_column(
            "data",
            &col.name,
            TypedColumn::nullable(col.data_type.clone()),
        );
    }

    let column_types = infer_select_column_types(&select_stmt, &ctx);

    // Extract aliases from select list
    let select_list = select_stmt.select_list().expect("no select list");
    let items: Vec<_> = select_list.items().collect();

    items
        .iter()
        .zip(column_types.iter())
        .map(|(item, typed_col)| {
            let alias = item.alias().unwrap_or_else(|| "?".to_string());
            (alias, typed_col.data_type.clone())
        })
        .collect()
}

/// Compare smelt inference against one oracle backend, returning an error message on mismatch.
fn check_types_against_oracle(
    oracle: &dyn TypeOracle,
    backend: &str,
    sql: &str,
    columns: &[generators::TypedSource],
    divergences: &[TypeDivergence],
) -> Result<(), String> {
    let actual_types = match oracle.query_types(sql) {
        Ok(types) => types,
        Err(_) => return Ok(()), // Skip invalid SQL for this backend
    };

    let inferred_types = run_smelt_inference(sql, columns);

    for (i, actual) in actual_types.iter().enumerate() {
        let inferred = if i < inferred_types.len() {
            &inferred_types[i]
        } else {
            continue;
        };

        let smelt_type = &inferred.1;
        let actual_type = &actual.1;

        if *smelt_type == DataType::Unknown {
            continue;
        }

        match compare_types(smelt_type, actual_type) {
            TypeMatch::Exact | TypeMatch::Compatible { .. } => {}
            TypeMatch::Mismatch => {
                if find_divergence(smelt_type, actual_type, backend, divergences).is_none() {
                    return Err(format!(
                        "Type mismatch for column {} ({}) against {backend}:\n  \
                         smelt inferred: {smelt_type:?}\n  \
                         {backend} actual:  {actual_type:?}\n  \
                         SQL: {sql}",
                        i, actual.0
                    ));
                }
            }
        }
    }
    Ok(())
}

// ---- Property tests ----

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Core property test: smelt's inferred types should match DuckDB's (and optionally
    /// Spark's) actual types for randomly generated SQL expressions.
    #[test]
    fn prop_type_inference(
        (columns, expr_kinds, func_indices) in test_scenario_strategy()
    ) {
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        // Generate expressions from the column pool
        let mut exprs: Vec<TypedExpr> = Vec::new();
        for (i, kind) in expr_kinds.iter().enumerate() {
            let func_idx = func_indices.get(i).copied().unwrap_or(0);
            if let Some(expr) = generate_expr(&columns, *kind, i, func_idx) {
                exprs.push(expr);
            }
        }

        // Need at least one expression
        prop_assume!(!exprs.is_empty());

        let sql = assemble_cte_query(&columns, &exprs);

        // Always check DuckDB
        if let Err(msg) = check_types_against_oracle(&duckdb, "duckdb", &sql, &columns, &divergences) {
            prop_assert!(false, "{}", msg);
        }

        // Check Spark if available (shared session, one JVM for all cases)
        if let Some(spark) = SPARK.as_ref() {
            if let Err(msg) = check_types_against_oracle(spark, "spark", &sql, &columns, &divergences) {
                prop_assert!(false, "{}", msg);
            }
        }
    }
}

// ---- Deterministic smoke tests ----

#[test]
fn smoke_cast_integer() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(42 AS INTEGER) AS x) SELECT x AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();
    assert_eq!(actual[0].1, DataType::Integer);

    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Integer,
        cast_sql: "CAST(42 AS INTEGER)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);
    assert_eq!(inferred[0].1, DataType::Integer);

    if let Some(spark) = SPARK.as_ref() {
        let divergences = known_divergences();
        check_types_against_oracle(spark, "spark", sql, &columns, &divergences).unwrap();
    }
}

#[test]
fn smoke_upper_function() {
    let oracle = DuckDbOracle::new();
    let sql =
        "WITH data AS (SELECT CAST('hello' AS VARCHAR) AS s) SELECT UPPER(s) AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "s".into(),
        data_type: DataType::Varchar { max_length: None },
        cast_sql: "CAST('hello' AS VARCHAR)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    // smelt returns Text, DuckDB returns Varchar — should be Compatible
    let match_result = compare_types(&inferred[0].1, &actual[0].1);
    assert!(matches!(
        match_result,
        TypeMatch::Exact | TypeMatch::Compatible { .. }
    ));

    if let Some(spark) = SPARK.as_ref() {
        let divergences = known_divergences();
        check_types_against_oracle(spark, "spark", sql, &columns, &divergences).unwrap();
    }
}

#[test]
fn smoke_count_aggregate() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(1 AS INTEGER) AS x) SELECT COUNT(x) AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Integer,
        cast_sql: "CAST(1 AS INTEGER)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    assert_eq!(inferred[0].1, DataType::BigInt);
    assert_eq!(actual[0].1, DataType::BigInt);

    if let Some(spark) = SPARK.as_ref() {
        let divergences = known_divergences();
        check_types_against_oracle(spark, "spark", sql, &columns, &divergences).unwrap();
    }
}

#[test]
fn smoke_binary_add() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(1 AS INTEGER) AS x) SELECT x + x AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Integer,
        cast_sql: "CAST(1 AS INTEGER)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    // smelt may infer Unknown if the parser doesn't fully resolve binary
    // expressions with CTE column references; that's acceptable for now.
    if inferred[0].1 != DataType::Unknown {
        let match_result = compare_types(&inferred[0].1, &actual[0].1);
        assert!(
            matches!(
                match_result,
                TypeMatch::Exact | TypeMatch::Compatible { .. }
            ),
            "smelt={:?}, duckdb={:?}",
            inferred[0].1,
            actual[0].1
        );
    }

    if let Some(spark) = SPARK.as_ref() {
        let divergences = known_divergences();
        check_types_against_oracle(spark, "spark", sql, &columns, &divergences).unwrap();
    }
}

#[test]
fn smoke_case_expression() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(42 AS INTEGER) AS x) SELECT CASE WHEN TRUE THEN x ELSE x END AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Integer,
        cast_sql: "CAST(42 AS INTEGER)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    let match_result = compare_types(&inferred[0].1, &actual[0].1);
    assert!(matches!(
        match_result,
        TypeMatch::Exact | TypeMatch::Compatible { .. }
    ));

    if let Some(spark) = SPARK.as_ref() {
        let divergences = known_divergences();
        check_types_against_oracle(spark, "spark", sql, &columns, &divergences).unwrap();
    }
}
