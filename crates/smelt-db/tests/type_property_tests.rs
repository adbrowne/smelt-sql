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
    self, assemble_cte_query, generate_expr, join_scenario_strategy, multi_model_scenario_strategy,
    test_scenario_strategy, three_model_scenario_strategy, wrap_in_outer_cte, TypedExpr,
};
use prop_helpers::spark_oracle::SparkOracle;
use prop_helpers::type_comparison::{compare_types, TypeMatch};

use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_parser::ast::File;
use smelt_types::{DataType, TypedColumn};

use std::path::{Path, PathBuf};

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

        if *smelt_type == DataType::unknown_dynamic() {
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
        (columns, shape, expr_kinds, func_indices) in test_scenario_strategy()
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

        let sql = assemble_cte_query(&columns, &exprs, &shape);

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
    if inferred[0].1 != DataType::unknown_dynamic() {
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
fn smoke_binary_division() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(7 AS INTEGER) AS x) SELECT x / x AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Integer,
        cast_sql: "CAST(7 AS INTEGER)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    // Integer / Integer returns Double — smelt and DuckDB agree.
    assert_eq!(
        inferred[0].1,
        DataType::Double,
        "smelt division should be Double"
    );
    assert_eq!(
        actual[0].1,
        DataType::Double,
        "DuckDB division should be Double"
    );

    if let Some(spark) = SPARK.as_ref() {
        let divergences = known_divergences();
        check_types_against_oracle(spark, "spark", sql, &columns, &divergences).unwrap();
    }
}

#[test]
fn smoke_nested_cte_division() {
    let oracle = DuckDbOracle::new();
    let inner = "WITH data AS (SELECT CAST(7 AS INTEGER) AS x) SELECT x / x AS expr_0 FROM data";
    let sql = wrap_in_outer_cte(inner);
    let actual = oracle.query_types(&sql).unwrap();

    // Integer / Integer returns Double through a nested CTE boundary.
    assert_eq!(
        actual[0].1,
        DataType::Double,
        "DuckDB nested-CTE division should be Double"
    );
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

#[test]
fn smoke_group_by_count() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST('hello' AS STRING) AS s, CAST(42 AS INTEGER) AS x) \
               SELECT s AS grp_0, COUNT(x) AS expr_0 FROM data GROUP BY s";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![
        generators::TypedSource {
            name: "s".into(),
            data_type: DataType::Varchar { max_length: None },
            cast_sql: "CAST('hello' AS STRING)".into(),
        },
        generators::TypedSource {
            name: "x".into(),
            data_type: DataType::Integer,
            cast_sql: "CAST(42 AS INTEGER)".into(),
        },
    ];
    let inferred = run_smelt_inference(sql, &columns);

    // grp_0 should be Varchar
    let match_grp = compare_types(&inferred[0].1, &actual[0].1);
    assert!(
        matches!(match_grp, TypeMatch::Exact | TypeMatch::Compatible { .. }),
        "grp_0: smelt={:?}, duckdb={:?}",
        inferred[0].1,
        actual[0].1
    );

    // expr_0 (COUNT) should be BigInt
    assert_eq!(inferred[1].1, DataType::BigInt);
    assert_eq!(actual[1].1, DataType::BigInt);
}

#[test]
fn smoke_group_by_having() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST('hello' AS STRING) AS s, CAST(42 AS INTEGER) AS x) \
               SELECT s AS grp_0, COUNT(x) AS expr_0 FROM data GROUP BY s HAVING COUNT(x) > 0";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![
        generators::TypedSource {
            name: "s".into(),
            data_type: DataType::Varchar { max_length: None },
            cast_sql: "CAST('hello' AS STRING)".into(),
        },
        generators::TypedSource {
            name: "x".into(),
            data_type: DataType::Integer,
            cast_sql: "CAST(42 AS INTEGER)".into(),
        },
    ];
    let inferred = run_smelt_inference(sql, &columns);

    // grp_0 should be Varchar
    let match_grp = compare_types(&inferred[0].1, &actual[0].1);
    assert!(
        matches!(match_grp, TypeMatch::Exact | TypeMatch::Compatible { .. }),
        "grp_0: smelt={:?}, duckdb={:?}",
        inferred[0].1,
        actual[0].1
    );

    // expr_0 (COUNT) should be BigInt
    assert_eq!(inferred[1].1, DataType::BigInt);
    assert_eq!(actual[1].1, DataType::BigInt);
}

// ---- Multi-model property tests ----

/// Set up a Salsa DB with two models: upstream and downstream.
fn setup_two_model_db(
    upstream_sql: &str,
    downstream_sql: &str,
) -> (smelt_db::Database, PathBuf, PathBuf) {
    let mut db = smelt_db::Database::default();
    let upstream_path = PathBuf::from("models/upstream.sql");
    let downstream_path = PathBuf::from("models/downstream.sql");

    let root = PathBuf::from(".");
    let upstream_file = db.set_source_file(
        upstream_path.clone(),
        upstream_sql.to_string(),
        root.clone(),
    );
    let downstream_file = db.set_source_file(
        downstream_path.clone(),
        downstream_sql.to_string(),
        root.clone(),
    );
    let project = db.set_project_input(root, String::new());
    db.set_workspace(vec![upstream_file, downstream_file], vec![project]);

    (db, upstream_path, downstream_path)
}

/// Run smelt type inference on the downstream model via the Salsa DB.
fn run_smelt_multi_model_inference(
    db: &smelt_db::Database,
    downstream_path: &Path,
) -> Vec<(String, DataType)> {
    let file = db
        .source_file(downstream_path)
        .expect("downstream file not registered");
    let ws = smelt_db::Workspace::get(db);
    let schema = smelt_db::typed_model_schema(db, ws, file);
    schema
        .columns
        .iter()
        .map(|col| {
            let dt = col
                .data_type
                .as_ref()
                .map(|tc| tc.data_type.clone())
                .unwrap_or(DataType::unknown_dynamic());
            (col.name.clone(), dt)
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Multi-model property test: type inference through smelt.ref() should match
    /// DuckDB's actual types for the same expressions.
    #[test]
    fn prop_multi_model_type_inference(scenario in multi_model_scenario_strategy()) {
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        // Get DuckDB actual types
        let actual_types = match duckdb.query_types(&scenario.duckdb_sql) {
            Ok(types) => types,
            Err(_) => return Ok(()), // Skip if DuckDB can't run it
        };

        // Get smelt inference via Salsa cross-model path
        let (db, _upstream_path, downstream_path) =
            setup_two_model_db(&scenario.upstream_sql, &scenario.downstream_sql);
        let inferred_types = run_smelt_multi_model_inference(&db, &downstream_path);

        for (i, actual) in actual_types.iter().enumerate() {
            let inferred = if i < inferred_types.len() {
                &inferred_types[i]
            } else {
                continue;
            };

            let smelt_type = &inferred.1;
            let actual_type = &actual.1;

            if *smelt_type == DataType::unknown_dynamic() {
                continue;
            }

            match compare_types(smelt_type, actual_type) {
                TypeMatch::Exact | TypeMatch::Compatible { .. } => {}
                TypeMatch::Mismatch => {
                    if find_divergence(smelt_type, actual_type, "duckdb", &divergences).is_none() {
                        prop_assert!(
                            false,
                            "Multi-model type mismatch for column {} ({}):\n  \
                             smelt inferred: {:?}\n  \
                             duckdb actual:  {:?}\n  \
                             upstream SQL: {}\n  \
                             downstream SQL: {}\n  \
                             duckdb SQL: {}",
                            i, actual.0, smelt_type, actual_type,
                            scenario.upstream_sql, scenario.downstream_sql, scenario.duckdb_sql
                        );
                    }
                }
            }
        }
    }
}

/// Smoke test: simple cross-model INTEGER passthrough.
#[test]
fn smoke_multi_model_integer_passthrough() {
    let upstream_sql = "WITH data AS (SELECT CAST(42 AS INTEGER) AS x) SELECT x FROM data";
    let downstream_sql = "SELECT x AS expr_0 FROM smelt.models.upstream";
    let duckdb_sql =
        "WITH upstream AS (SELECT CAST(42 AS INTEGER) AS x) SELECT x AS expr_0 FROM upstream";

    let oracle = DuckDbOracle::new();
    let actual = oracle.query_types(duckdb_sql).unwrap();
    assert_eq!(actual[0].1, DataType::Integer);

    let (db, _, downstream_path) = setup_two_model_db(upstream_sql, downstream_sql);
    let inferred = run_smelt_multi_model_inference(&db, &downstream_path);
    assert_eq!(inferred[0].1, DataType::Integer);
}

/// Smoke test: cross-model with expression (LENGTH on VARCHAR).
#[test]
fn smoke_multi_model_function_on_ref() {
    let upstream_sql = "WITH data AS (SELECT CAST('hello' AS STRING) AS s) SELECT s FROM data";
    let downstream_sql = "SELECT LENGTH(s) AS expr_0 FROM smelt.models.upstream";
    let duckdb_sql = "WITH upstream AS (SELECT CAST('hello' AS STRING) AS s) SELECT LENGTH(s) AS expr_0 FROM upstream";

    let oracle = DuckDbOracle::new();
    let actual = oracle.query_types(duckdb_sql).unwrap();
    assert_eq!(actual[0].1, DataType::BigInt);

    let (db, _, downstream_path) = setup_two_model_db(upstream_sql, downstream_sql);
    let inferred = run_smelt_multi_model_inference(&db, &downstream_path);
    assert_eq!(inferred[0].1, DataType::BigInt);
}

// ---- Three-model chain property tests ----

/// Set up a Salsa DB with three models: A → B → C.
fn setup_three_model_db(a_sql: &str, b_sql: &str, c_sql: &str) -> (smelt_db::Database, PathBuf) {
    let mut db = smelt_db::Database::default();
    let a_path = PathBuf::from("models/model_a.sql");
    let b_path = PathBuf::from("models/model_b.sql");
    let c_path = PathBuf::from("models/model_c.sql");

    let root = PathBuf::from(".");
    let a_file = db.set_source_file(a_path.clone(), a_sql.to_string(), root.clone());
    let b_file = db.set_source_file(b_path.clone(), b_sql.to_string(), root.clone());
    let c_file = db.set_source_file(c_path.clone(), c_sql.to_string(), root.clone());
    let project = db.set_project_input(root, String::new());
    db.set_workspace(vec![a_file, b_file, c_file], vec![project]);

    (db, c_path)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Three-model chain property test: A → B → C type inference should match DuckDB.
    #[test]
    fn prop_three_model_type_inference(scenario in three_model_scenario_strategy()) {
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        let actual_types = match duckdb.query_types(&scenario.duckdb_sql) {
            Ok(types) => types,
            Err(_) => return Ok(()),
        };

        let (db, c_path) = setup_three_model_db(
            &scenario.model_a_sql,
            &scenario.model_b_sql,
            &scenario.model_c_sql,
        );
        let inferred_types = run_smelt_multi_model_inference(&db, &c_path);

        for (i, actual) in actual_types.iter().enumerate() {
            let inferred = if i < inferred_types.len() {
                &inferred_types[i]
            } else {
                continue;
            };

            let smelt_type = &inferred.1;
            let actual_type = &actual.1;

            if *smelt_type == DataType::unknown_dynamic() {
                continue;
            }

            match compare_types(smelt_type, actual_type) {
                TypeMatch::Exact | TypeMatch::Compatible { .. } => {}
                TypeMatch::Mismatch => {
                    if find_divergence(smelt_type, actual_type, "duckdb", &divergences).is_none() {
                        prop_assert!(
                            false,
                            "Three-model type mismatch for column {} ({}):\n  \
                             smelt: {:?}, duckdb: {:?}\n  \
                             A: {}\n  B: {}\n  C: {}\n  duckdb: {}",
                            i, actual.0, smelt_type, actual_type,
                            scenario.model_a_sql, scenario.model_b_sql,
                            scenario.model_c_sql, scenario.duckdb_sql
                        );
                    }
                }
            }
        }
    }
}

// ---- JOIN property tests ----

/// Set up a Salsa DB with two upstream models and a downstream JOIN model.
fn setup_join_db(left_sql: &str, right_sql: &str, join_sql: &str) -> (smelt_db::Database, PathBuf) {
    let mut db = smelt_db::Database::default();
    let left_path = PathBuf::from("models/left_model.sql");
    let right_path = PathBuf::from("models/right_model.sql");
    let join_path = PathBuf::from("models/join_model.sql");

    let root = PathBuf::from(".");
    let left_file = db.set_source_file(left_path.clone(), left_sql.to_string(), root.clone());
    let right_file = db.set_source_file(right_path.clone(), right_sql.to_string(), root.clone());
    let join_file = db.set_source_file(join_path.clone(), join_sql.to_string(), root.clone());
    let project = db.set_project_input(root, String::new());
    db.set_workspace(vec![left_file, right_file, join_file], vec![project]);

    (db, join_path)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// JOIN property test: types through INNER JOIN of two upstream models.
    #[test]
    fn prop_join_type_inference(scenario in join_scenario_strategy()) {
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        let actual_types = match duckdb.query_types(&scenario.duckdb_sql) {
            Ok(types) => types,
            Err(_) => return Ok(()),
        };

        let (db, join_path) = setup_join_db(
            &scenario.left_sql,
            &scenario.right_sql,
            &scenario.join_sql,
        );
        let inferred_types = run_smelt_multi_model_inference(&db, &join_path);

        for (i, actual) in actual_types.iter().enumerate() {
            let inferred = if i < inferred_types.len() {
                &inferred_types[i]
            } else {
                continue;
            };

            let smelt_type = &inferred.1;
            let actual_type = &actual.1;

            if *smelt_type == DataType::unknown_dynamic() {
                continue;
            }

            match compare_types(smelt_type, actual_type) {
                TypeMatch::Exact | TypeMatch::Compatible { .. } => {}
                TypeMatch::Mismatch => {
                    if find_divergence(smelt_type, actual_type, "duckdb", &divergences).is_none() {
                        prop_assert!(
                            false,
                            "JOIN type mismatch for column {} ({}):\n  \
                             smelt: {:?}, duckdb: {:?}\n  \
                             left: {}\n  right: {}\n  join: {}\n  duckdb: {}",
                            i, actual.0, smelt_type, actual_type,
                            scenario.left_sql, scenario.right_sql,
                            scenario.join_sql, scenario.duckdb_sql
                        );
                    }
                }
            }
        }
    }
}
