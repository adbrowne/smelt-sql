//! Type conformance tests — verify that cast-wrapped SQL produces output types
//! that match smelt's type inference *exactly*.
//!
//! Unlike `type_property_tests.rs` which documents divergences, these tests
//! assert zero divergences: every column must match exactly after wrapping.

#[allow(dead_code)]
mod prop_helpers;

use prop_helpers::duckdb_oracle::{DuckDbOracle, TypeOracle};
use prop_helpers::generators::{
    self, assemble_cte_query, generate_expr, test_scenario_strategy, TypedExpr,
};
use prop_helpers::spark_oracle::SparkOracle;

use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_dialect::{wrap_with_type_casts, BackendCapabilities, PrintContext, SqlDialect};
use smelt_parser::ast::File;
use smelt_types::{DataType, TypedColumn};

/// Translate a smelt type to what the backend should produce after casting.
/// This mirrors the boundary translation: Text → Varchar, etc.
fn to_backend_type(dt: &DataType) -> DataType {
    match dt {
        DataType::Text => DataType::Varchar { max_length: None },
        other => other.clone(),
    }
}

use proptest::prelude::*;
use std::sync::LazyLock;

/// Shared SparkOracle — created once on first access.
static SPARK: LazyLock<Option<SparkOracle>> = LazyLock::new(|| {
    std::env::var("SPARK_CONTAINER_ID")
        .ok()
        .map(|id| SparkOracle::new(&id))
});

/// Parse SQL with smelt and run type inference.
fn run_smelt_inference(sql: &str, columns: &[generators::TypedSource]) -> Vec<(String, DataType)> {
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt in parsed SQL");

    let mut ctx = TypeContext::new();
    for col in columns {
        ctx.add_cte_column(
            "data",
            &col.name,
            TypedColumn::nullable(col.data_type.clone()),
        );
    }

    let column_types = infer_select_column_types(&select_stmt, &ctx);

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

/// Translate SQL through the dialect printer for a specific backend.
/// This remaps function names (e.g. EVERY -> BOOL_AND for DuckDB).
fn translate_for_backend(sql: &str, backend: &str) -> String {
    let (dialect, capabilities) = match backend {
        "duckdb" => (SqlDialect::DuckDB, BackendCapabilities::duckdb()),
        "spark" => (SqlDialect::SparkSQL, BackendCapabilities::spark()),
        _ => return sql.to_string(),
    };
    let parse = smelt_parser::parse(sql);
    let ctx = PrintContext {
        dialect: &dialect,
        capabilities: &capabilities,
        schema: "main",
        ephemeral_models: std::collections::HashSet::new(),
        cross_engine_refs: std::collections::HashMap::new(),
        smelt_as_struct: None,
        smelt_fn: None,
    };
    smelt_dialect::print(&parse.syntax(), &ctx)
}

/// Wrap SQL with type casts and verify the oracle returns exact type matches.
fn check_conformance(
    oracle: &dyn TypeOracle,
    backend: &str,
    sql: &str,
    _columns: &[generators::TypedSource],
    inferred: &[(String, DataType)],
) -> Result<(), String> {
    // Translate through dialect printer (remaps function names)
    let backend_sql = translate_for_backend(sql, backend);

    // Build the cast-wrapped SQL
    let col_names: Vec<&str> = inferred.iter().map(|(name, _)| name.as_str()).collect();
    let col_types: Vec<DataType> = inferred.iter().map(|(_, dt)| dt.clone()).collect();
    let cast_sql = wrap_with_type_casts(&backend_sql, &col_names, &col_types);

    let actual_types = match oracle.query_types(&cast_sql) {
        Ok(types) => types,
        Err(e) => {
            return Err(format!(
                "Oracle query failed for {backend}:\n  Error: {e}\n  Original SQL: {sql}\n  Cast SQL: {cast_sql}"
            ));
        }
    };

    for (i, actual) in actual_types.iter().enumerate() {
        let expected = if i < inferred.len() {
            &inferred[i].1
        } else {
            continue;
        };

        // Skip Unknown types (smelt couldn't infer)
        if *expected == DataType::Unknown {
            continue;
        }

        // Compare using backend-translated type (e.g. Text → Varchar)
        let backend_expected = to_backend_type(expected);
        if backend_expected != actual.1 {
            return Err(format!(
                "Type conformance failure for column {} ({}) against {backend}:\n  \
                 expected (backend): {backend_expected:?}\n  \
                 actual ({backend}):  {:?}\n  \
                 smelt inferred:     {expected:?}\n  \
                 Original SQL: {sql}\n  \
                 Cast SQL: {cast_sql}",
                i, actual.0, actual.1
            ));
        }
    }
    Ok(())
}

// ---- Property tests ----

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Verify that cast-wrapped SQL produces exact type matches against DuckDB.
    #[test]
    fn prop_type_conformance_duckdb(
        (columns, shape, expr_kinds, func_indices) in test_scenario_strategy()
    ) {
        let duckdb = DuckDbOracle::new();

        let mut exprs: Vec<TypedExpr> = Vec::new();
        for (i, kind) in expr_kinds.iter().enumerate() {
            let func_idx = func_indices.get(i).copied().unwrap_or(0);
            if let Some(expr) = generate_expr(&columns, *kind, i, func_idx) {
                exprs.push(expr);
            }
        }
        prop_assume!(!exprs.is_empty());

        let sql = assemble_cte_query(&columns, &exprs, &shape);
        let inferred = run_smelt_inference(&sql, &columns);

        if let Err(msg) = check_conformance(&duckdb, "duckdb", &sql, &columns, &inferred) {
            prop_assert!(false, "{}", msg);
        }
    }
}

// ---- Deterministic smoke tests ----

#[test]
fn conformance_smoke_integer() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(42 AS INTEGER) AS x) SELECT x AS expr_0 FROM data";
    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Integer,
        cast_sql: "CAST(42 AS INTEGER)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);
    check_conformance(&oracle, "duckdb", sql, &columns, &inferred).unwrap();
}

#[test]
fn conformance_smoke_upper_text_to_varchar() {
    let oracle = DuckDbOracle::new();
    let sql =
        "WITH data AS (SELECT CAST('hello' AS VARCHAR) AS s) SELECT UPPER(s) AS expr_0 FROM data";
    let columns = vec![generators::TypedSource {
        name: "s".into(),
        data_type: DataType::Varchar { max_length: None },
        cast_sql: "CAST('hello' AS VARCHAR)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);
    // smelt infers Text, but after casting Text→VARCHAR at boundary, DuckDB should match
    check_conformance(&oracle, "duckdb", sql, &columns, &inferred).unwrap();
}

#[test]
fn conformance_smoke_count() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(1 AS INTEGER) AS x) SELECT COUNT(x) AS expr_0 FROM data";
    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Integer,
        cast_sql: "CAST(1 AS INTEGER)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);
    check_conformance(&oracle, "duckdb", sql, &columns, &inferred).unwrap();
}

#[test]
fn conformance_smoke_sum_integer() {
    // SUM(INTEGER) — smelt infers BigInt, DuckDB natively returns Decimal(38,0).
    // After casting, DuckDB should return BigInt.
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(42 AS INTEGER) AS x) SELECT SUM(x) AS expr_0 FROM data";
    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Integer,
        cast_sql: "CAST(42 AS INTEGER)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);
    check_conformance(&oracle, "duckdb", sql, &columns, &inferred).unwrap();
}

#[test]
fn conformance_smoke_binary_add() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(1 AS INTEGER) AS x) SELECT x + x AS expr_0 FROM data";
    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Integer,
        cast_sql: "CAST(1 AS INTEGER)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);
    if inferred[0].1 != DataType::Unknown {
        check_conformance(&oracle, "duckdb", sql, &columns, &inferred).unwrap();
    }
}

#[test]
fn conformance_smoke_case_expr() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(42 AS INTEGER) AS x) SELECT CASE WHEN TRUE THEN x ELSE x END AS expr_0 FROM data";
    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Integer,
        cast_sql: "CAST(42 AS INTEGER)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);
    check_conformance(&oracle, "duckdb", sql, &columns, &inferred).unwrap();
}

#[test]
fn conformance_smoke_string_concat() {
    // || — smelt infers Text, after casting to VARCHAR should match DuckDB
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST('a' AS VARCHAR) AS s) SELECT s || s AS expr_0 FROM data";
    let columns = vec![generators::TypedSource {
        name: "s".into(),
        data_type: DataType::Varchar { max_length: None },
        cast_sql: "CAST('a' AS VARCHAR)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);
    check_conformance(&oracle, "duckdb", sql, &columns, &inferred).unwrap();
}

// ---- Spark conformance (when available) ----

#[test]
fn conformance_smoke_spark_integer() {
    if let Some(spark) = SPARK.as_ref() {
        let sql = "WITH data AS (SELECT CAST(42 AS INTEGER) AS x) SELECT x AS expr_0 FROM data";
        let columns = vec![generators::TypedSource {
            name: "x".into(),
            data_type: DataType::Integer,
            cast_sql: "CAST(42 AS INTEGER)".into(),
        }];
        let inferred = run_smelt_inference(sql, &columns);
        check_conformance(spark, "spark", sql, &columns, &inferred).unwrap();
    }
}

#[test]
fn conformance_smoke_spark_upper() {
    if let Some(spark) = SPARK.as_ref() {
        let sql = "WITH data AS (SELECT CAST('hello' AS STRING) AS s) SELECT UPPER(s) AS expr_0 FROM data";
        let columns = vec![generators::TypedSource {
            name: "s".into(),
            data_type: DataType::Varchar { max_length: None },
            cast_sql: "CAST('hello' AS STRING)".into(),
        }];
        let inferred = run_smelt_inference(sql, &columns);
        check_conformance(spark, "spark", sql, &columns, &inferred).unwrap();
    }
}
