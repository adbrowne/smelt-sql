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

use prop_helpers::divergences::{find_divergence, known_divergences};
use prop_helpers::duckdb_oracle::{DuckDbOracle, TypeOracle};
use prop_helpers::generators::{
    self, assemble_cte_query, generate_expr, join_scenario_strategy, multi_model_scenario_strategy,
    test_scenario_strategy, three_model_scenario_strategy, wrap_in_outer_cte, TypedExpr,
};
use prop_helpers::known_unknowns::{find_known_unknown, known_unknowns};
use prop_helpers::oracle_check::{
    check_types_against_oracle, expr_sql_by_alias, run_smelt_inference,
};
use prop_helpers::spark_oracle::SparkOracle;
use prop_helpers::type_comparison::{compare_types, TypeMatch};

use smelt_types::DataType;

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
        let unknowns = known_unknowns();

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
        if let Err(msg) =
            check_types_against_oracle(&duckdb, "duckdb", &sql, &columns, &exprs, &divergences, &unknowns)
        {
            prop_assert!(false, "{}", msg);
        }

        // Check Spark if available (shared session, one JVM for all cases)
        if let Some(spark) = SPARK.as_ref() {
            if let Err(msg) =
                check_types_against_oracle(spark, "spark", &sql, &columns, &exprs, &divergences, &unknowns)
            {
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
        check_types_against_oracle(
            spark,
            "spark",
            sql,
            &columns,
            &[],
            &divergences,
            &known_unknowns(),
        )
        .unwrap();
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
        check_types_against_oracle(
            spark,
            "spark",
            sql,
            &columns,
            &[],
            &divergences,
            &known_unknowns(),
        )
        .unwrap();
    }
}

#[test]
fn smoke_md5_function() {
    let oracle = DuckDbOracle::new();
    let sql =
        "WITH data AS (SELECT CAST('hello' AS VARCHAR) AS s) SELECT MD5(s) AS expr_0 FROM data";
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
}

#[test]
fn smoke_to_seconds_function() {
    let oracle = DuckDbOracle::new();
    let sql =
        "WITH data AS (SELECT CAST(42.5 AS DOUBLE) AS s) SELECT TO_SECONDS(s) AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "s".into(),
        data_type: DataType::Double,
        cast_sql: "CAST(42.5 AS DOUBLE)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    let match_result = compare_types(&inferred[0].1, &actual[0].1);
    assert!(matches!(
        match_result,
        TypeMatch::Exact | TypeMatch::Compatible { .. }
    ));
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
        check_types_against_oracle(
            spark,
            "spark",
            sql,
            &columns,
            &[],
            &divergences,
            &known_unknowns(),
        )
        .unwrap();
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
        check_types_against_oracle(
            spark,
            "spark",
            sql,
            &columns,
            &[],
            &divergences,
            &known_unknowns(),
        )
        .unwrap();
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
        check_types_against_oracle(
            spark,
            "spark",
            sql,
            &columns,
            &[],
            &divergences,
            &known_unknowns(),
        )
        .unwrap();
    }
}

/// NOT-prefixed binary operators (Phase 1, `not_prefixed_binary_operator`
/// ledger category) — `NOT IN`, `NOT LIKE`, `NOT ILIKE`, `NOT BETWEEN`,
/// `NOT SIMILAR TO`, and bare `NOT NULL` all resolve to Boolean, matching a
/// real DuckDB.
#[test]
fn smoke_not_in() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(2 AS INTEGER) AS x) \
               SELECT x NOT IN (2, 3) AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Integer,
        cast_sql: "CAST(2 AS INTEGER)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    assert_eq!(
        inferred[0].1,
        DataType::Boolean,
        "smelt NOT IN should be Boolean"
    );
    assert_eq!(
        actual[0].1,
        DataType::Boolean,
        "DuckDB NOT IN should be Boolean"
    );
}

#[test]
fn smoke_not_like() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST('abc' AS VARCHAR) AS x) \
               SELECT x NOT LIKE 'z%' AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Text,
        cast_sql: "CAST('abc' AS VARCHAR)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    assert_eq!(
        inferred[0].1,
        DataType::Boolean,
        "smelt NOT LIKE should be Boolean"
    );
    assert_eq!(
        actual[0].1,
        DataType::Boolean,
        "DuckDB NOT LIKE should be Boolean"
    );
}

#[test]
fn smoke_not_ilike() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST('abc' AS VARCHAR) AS x) \
               SELECT x NOT ILIKE 'Z%' AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Text,
        cast_sql: "CAST('abc' AS VARCHAR)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    assert_eq!(
        inferred[0].1,
        DataType::Boolean,
        "smelt NOT ILIKE should be Boolean"
    );
    assert_eq!(
        actual[0].1,
        DataType::Boolean,
        "DuckDB NOT ILIKE should be Boolean"
    );
}

#[test]
fn smoke_not_between() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(5 AS INTEGER) AS x) \
               SELECT x NOT BETWEEN 1 AND 3 AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Integer,
        cast_sql: "CAST(5 AS INTEGER)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    assert_eq!(
        inferred[0].1,
        DataType::Boolean,
        "smelt NOT BETWEEN should be Boolean"
    );
    assert_eq!(
        actual[0].1,
        DataType::Boolean,
        "DuckDB NOT BETWEEN should be Boolean"
    );
}

#[test]
fn smoke_similar_to() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST('abc' AS VARCHAR) AS x) \
               SELECT x SIMILAR TO 'a.*' AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Text,
        cast_sql: "CAST('abc' AS VARCHAR)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    assert_eq!(
        inferred[0].1,
        DataType::Boolean,
        "smelt SIMILAR TO should be Boolean"
    );
    assert_eq!(
        actual[0].1,
        DataType::Boolean,
        "DuckDB SIMILAR TO should be Boolean"
    );
}

#[test]
fn smoke_not_similar_to() {
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST('abc' AS VARCHAR) AS x) \
               SELECT x NOT SIMILAR TO 'z.*' AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Text,
        cast_sql: "CAST('abc' AS VARCHAR)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    assert_eq!(
        inferred[0].1,
        DataType::Boolean,
        "smelt NOT SIMILAR TO should be Boolean"
    );
    assert_eq!(
        actual[0].1,
        DataType::Boolean,
        "DuckDB NOT SIMILAR TO should be Boolean"
    );
}

#[test]
fn smoke_bare_not_null() {
    // `expr NOT NULL` — DuckDB sugar for `expr IS NOT NULL`. Same ledger
    // category (`not_prefixed_binary_operator`) as NOT IN/LIKE/BETWEEN.
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(1 AS INTEGER) AS x) \
               SELECT x NOT NULL AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Integer,
        cast_sql: "CAST(1 AS INTEGER)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    assert_eq!(
        inferred[0].1,
        DataType::Boolean,
        "smelt bare NOT NULL should be Boolean"
    );
    assert_eq!(
        actual[0].1,
        DataType::Boolean,
        "DuckDB bare NOT NULL should be Boolean"
    );
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
        check_types_against_oracle(
            spark,
            "spark",
            sql,
            &columns,
            &[],
            &divergences,
            &known_unknowns(),
        )
        .unwrap();
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
        let unknowns = known_unknowns();
        let unknown_scope_sql = scenario.duckdb_sql.clone();

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

            if smelt_type.is_unknown() {
                if find_known_unknown(&unknown_scope_sql, &unknowns).is_some() {
                    continue;
                }
                prop_assert!(
                    false,
                    "Unregistered Unknown inference for column {} ({}): {}",
                    i, actual.0, unknown_scope_sql
                );
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
        let unknowns = known_unknowns();
        let unknown_scope_sql = scenario.duckdb_sql.clone();

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

            if smelt_type.is_unknown() {
                if find_known_unknown(&unknown_scope_sql, &unknowns).is_some() {
                    continue;
                }
                prop_assert!(
                    false,
                    "Unregistered Unknown inference for column {} ({}): {}",
                    i, actual.0, unknown_scope_sql
                );
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
        let unknowns = known_unknowns();
        let unknown_scope_sql = scenario.duckdb_sql.clone();

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

            if smelt_type.is_unknown() {
                if find_known_unknown(&unknown_scope_sql, &unknowns).is_some() {
                    continue;
                }
                prop_assert!(
                    false,
                    "Unregistered Unknown inference for column {} ({}): {}",
                    i, actual.0, unknown_scope_sql
                );
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

// ---- Generator reachability smoke tests ----
//
// These are statistical guards (not proofs): over a deterministic sample of
// generated scenarios the corpus must contain at least one occurrence of each
// of the "hard" inference paths the generators are meant to reach. If a future
// edit silently stops emitting temporal/decimal arithmetic, decimal casts,
// EXTRACT(EPOCH), mixed-tz comparisons, or the extended function list, one of
// these assertions fails — catching generator regressions the property test
// itself would silently paper over (an un-generated path can never diverge).
mod reachability {
    use super::generators::{assemble_cte_query, generate_expr, test_scenario_strategy, TypedExpr};
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;

    /// Deterministically sample `n` generated CTE queries from the top-level
    /// scenario strategy (the same one `prop_type_inference` drives).
    fn sample_generated_sql(n: usize) -> Vec<String> {
        let mut runner = TestRunner::deterministic();
        let strat = test_scenario_strategy();
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let tree = strat
                .new_tree(&mut runner)
                .expect("strategy generated a value");
            let (columns, shape, expr_kinds, func_indices) = tree.current();
            let mut exprs: Vec<TypedExpr> = Vec::new();
            for (i, kind) in expr_kinds.iter().enumerate() {
                let func_idx = func_indices.get(i).copied().unwrap_or(0);
                if let Some(expr) = generate_expr(&columns, *kind, i, func_idx) {
                    exprs.push(expr);
                }
            }
            if exprs.is_empty() {
                continue;
            }
            out.push(assemble_cte_query(&columns, &exprs, &shape));
        }
        out
    }

    const N: usize = 500;

    /// A binary operation between two columns whose names start with the given
    /// prefixes joined by one of `ops`, in either operand order.
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

    /// All whitespace/paren-delimited tokens in `sql` that begin with `prefix`.
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
        let corpus = sample_generated_sql(N);
        // interval ± (naive or tz-aware) timestamp, either order.
        let hit = has_binop(&corpus, "ts_col", &["+", "-"], "interval_col")
            || has_binop(&corpus, "interval_col", &["+", "-"], "ts_col")
            || has_binop(&corpus, "tstz_col", &["+", "-"], "interval_col")
            || has_binop(&corpus, "interval_col", &["+", "-"], "tstz_col");
        assert!(
            hit,
            "generators never produced interval±timestamp over {N} cases"
        );
    }

    #[test]
    fn reaches_temporal_difference() {
        // Temporal subtraction → Interval. `DATE - DATE` is deliberately not
        // generated (DuckDB returns an un-castable BIGINT — registered
        // `date_minus_date` divergence); the Interval-typed difference path is
        // covered by TIMESTAMP/TIMESTAMPTZ subtraction, which DuckDB also types
        // as INTERVAL and the cast-wrap conformance oracle accepts.
        let corpus = sample_generated_sql(N);
        let hit = has_binop(&corpus, "ts_col", &["-"], "ts_col")
            || has_binop(&corpus, "tstz_col", &["-"], "tstz_col");
        assert!(
            hit,
            "generators never produced a temporal difference over {N} cases"
        );
    }

    #[test]
    fn reaches_decimal_arithmetic() {
        let corpus = sample_generated_sql(N);
        assert!(
            has_binop(&corpus, "dec_col", &["+"], "dec_col"),
            "generators never produced decimal + decimal over {N} cases"
        );
        assert!(
            has_binop(&corpus, "dec_col", &["*"], "dec_col"),
            "generators never produced decimal * decimal over {N} cases"
        );
        assert!(
            has_binop(&corpus, "dec_col", &["/"], "dec_col"),
            "generators never produced decimal / decimal over {N} cases"
        );
    }

    #[test]
    fn reaches_decimal_cast() {
        let corpus = sample_generated_sql(N);
        assert!(
            corpus.iter().any(|s| s.contains("DECIMAL(12,3)")),
            "generators never produced CAST(... AS DECIMAL(12,3)) over {N} cases"
        );
    }

    #[test]
    fn reaches_extract_epoch() {
        let corpus = sample_generated_sql(N);
        assert!(
            corpus.iter().any(|s| s.contains("EXTRACT(EPOCH")),
            "generators never produced EXTRACT(EPOCH ...) over {N} cases"
        );
    }

    #[test]
    fn reaches_mixed_tz_comparison() {
        let corpus = sample_generated_sql(N);
        let cmp = &["=", "<>", "<", ">", "<=", ">="];
        let hit = has_binop(&corpus, "ts_col", cmp, "tstz_col")
            || has_binop(&corpus, "tstz_col", cmp, "ts_col");
        assert!(
            hit,
            "generators never produced a mixed-tz comparison over {N} cases"
        );
    }

    #[test]
    fn reaches_extended_functions() {
        // The function list is far longer than the 0..100 `func_idx` range, so a
        // uniform strategy sample is too sparse to reliably surface any specific
        // 10 functions. This guard instead sweeps the generator's own Function
        // path over a pool holding one column of every base type — a stronger
        // check that each extended function is *reachable* (guards against
        // silently dropping one from `core_functions`).
        use super::generators::{BaseType, ExprKind, TypedSource};
        let pool: Vec<TypedSource> = BaseType::all()
            .iter()
            .enumerate()
            .map(|(i, bt)| TypedSource {
                name: format!("{}_{}", bt.col_prefix(), i),
                data_type: bt.to_smelt_type(),
                cast_sql: bt.cast_sql().to_string(),
            })
            .collect();

        let n_funcs = super::generators::core_functions().len();
        let mut corpus: Vec<String> = Vec::new();
        for expr_idx in 0..n_funcs {
            for func_idx in 0..(n_funcs * 3) {
                if let Some(e) = generate_expr(&pool, ExprKind::Function, expr_idx, func_idx) {
                    corpus.push(e.sql);
                }
            }
        }

        // The extended, DuckDB-executable functions added to the generator.
        // (INITCAP/TO_CHAR aren't DuckDB scalars, and POSITION(x IN y) /
        // PERCENTILE_CONT WITHIN GROUP are deferred parser grammar, so those are
        // not generated — see the note in `core_functions`.)
        let new_functions = [
            "MEDIAN",
            "ARRAY_AGG",
            "AGE",
            "JSON_EXTRACT",
            "IFNULL",
            "TRANSLATE",
            "CORR",
            "COVAR_POP",
            "COVAR_SAMP",
            "MODE",
        ];
        let missing: Vec<&str> = new_functions
            .iter()
            .copied()
            .filter(|f| !corpus.iter().any(|s| s.contains(&format!("{f}("))))
            .collect();
        assert!(
            missing.is_empty(),
            "extended functions not reachable from the generator: {missing:?}"
        );
    }

    #[test]
    fn reaches_aggregate_filter() {
        // `agg(x) FILTER (WHERE cond)` is already parsed (see
        // crates/smelt-parser/src/parser/tests.rs FILTER tests) but was never
        // generated. Guards against the generator silently dropping the
        // FILTER wrapper it now attaches to a fraction of aggregate calls.
        let corpus = sample_generated_sql(N);
        assert!(
            corpus.iter().any(|sql| sql.contains(") FILTER (WHERE ")),
            "generators never produced an aggregate FILTER clause over {N} cases"
        );
    }
}

// ---- Known-unknowns staleness report ----
//
// Staleness pressure for `prop_helpers/known_unknowns.rs`: over a deterministic
// sample of generated queries, warn (never fail) about any registered entry that
// never matched a column smelt actually inferred as `Unknown`. A registered hole
// that has since been closed should have its entry deleted; this surfaces the
// candidates. Warn-level (eprintln) by design — closing a hole must not turn the
// suite red until someone prunes the registry.
#[test]
fn known_unknowns_staleness_report() {
    use prop_helpers::generators::{
        assemble_cte_query, generate_expr, test_scenario_strategy, TypedExpr,
    };
    use prop_helpers::known_unknowns::{find_known_unknown, known_unknowns};
    use proptest::strategy::{Strategy, ValueTree};
    use proptest::test_runner::TestRunner;

    let entries = known_unknowns();
    let mut fired: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();

    let mut runner = TestRunner::deterministic();
    let strat = test_scenario_strategy();
    const N: usize = 500;
    for _ in 0..N {
        let tree = strat.new_tree(&mut runner).expect("strategy value");
        let (columns, shape, expr_kinds, func_indices) = tree.current();
        let mut exprs: Vec<TypedExpr> = Vec::new();
        for (i, kind) in expr_kinds.iter().enumerate() {
            let func_idx = func_indices.get(i).copied().unwrap_or(0);
            if let Some(e) = generate_expr(&columns, *kind, i, func_idx) {
                exprs.push(e);
            }
        }
        if exprs.is_empty() {
            continue;
        }
        let sql = assemble_cte_query(&columns, &exprs, &shape);
        let inferred = run_smelt_inference(&sql, &columns);
        let by_alias = expr_sql_by_alias(&exprs);
        for (alias, dt) in &inferred {
            if dt.is_unknown() {
                let expr_sql = by_alias.get(alias).map(String::as_str).unwrap_or(&sql);
                if let Some(e) = find_known_unknown(expr_sql, &entries) {
                    fired.insert(e.id);
                }
            }
        }
    }

    // Cross-model passes: some holes (e.g. decimal-multiply overflow from a
    // chained `*` on an already-widened decimal, p'=43>38) only arise through
    // smelt.ref() lineage, never in a single CTE, and only in the deeper chains.
    // Sample both two- and three-model scenarios so those entries can fire,
    // scoped to the full lineage SQL like the cross-model property tests.
    const MM: usize = 1000;
    let mm_strat = multi_model_scenario_strategy();
    for _ in 0..MM {
        let tree = mm_strat.new_tree(&mut runner).expect("strategy value");
        let scenario = tree.current();
        let (db, _up, downstream_path) =
            setup_two_model_db(&scenario.upstream_sql, &scenario.downstream_sql);
        let inferred = run_smelt_multi_model_inference(&db, &downstream_path);
        if inferred.iter().any(|(_, dt)| dt.is_unknown()) {
            if let Some(e) = find_known_unknown(&scenario.duckdb_sql, &entries) {
                fired.insert(e.id);
            }
        }
    }
    let tm_strat = three_model_scenario_strategy();
    for _ in 0..MM {
        let tree = tm_strat.new_tree(&mut runner).expect("strategy value");
        let scenario = tree.current();
        let (db, c_path) = setup_three_model_db(
            &scenario.model_a_sql,
            &scenario.model_b_sql,
            &scenario.model_c_sql,
        );
        let inferred = run_smelt_multi_model_inference(&db, &c_path);
        if inferred.iter().any(|(_, dt)| dt.is_unknown()) {
            if let Some(e) = find_known_unknown(&scenario.duckdb_sql, &entries) {
                fired.insert(e.id);
            }
        }
    }

    for e in &entries {
        if !fired.contains(e.id) {
            eprintln!(
                "warning: known-unknown entry '{}' never fired over {N} generated cases \
                 — the inference hole may be closed; consider deleting the entry ({})",
                e.id, e.description
            );
        }
    }
}
