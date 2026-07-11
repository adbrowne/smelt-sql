//! Property-based tests for subquery type inference.
//!
//! Tests that smelt correctly infers types for:
//! - Scalar subqueries in SELECT and WHERE positions
//! - IN (SELECT ...) expressions
//! - EXISTS (SELECT ...) expressions
//!
//! Validated against DuckDB.

use crate::prop_helpers::divergences::{find_divergence, known_divergences, TypeDivergence};
use crate::prop_helpers::duckdb_oracle::{DuckDbOracle, TypeOracle};
use crate::prop_helpers::type_comparison::{compare_types, TypeMatch};

use smelt_db::type_inference::{build_subquery_context, infer_select_column_types, TypeContext};
use smelt_parser::ast::File;
use smelt_types::DataType;

use proptest::prelude::*;

// ---- Column type variants ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BaseType {
    SmallInt,
    Integer,
    BigInt,
    Float,
    Double,
    Decimal,
    Varchar,
    Boolean,
    Date,
    Timestamp,
}

impl BaseType {
    fn all() -> &'static [BaseType] {
        &[
            BaseType::SmallInt,
            BaseType::Integer,
            BaseType::BigInt,
            BaseType::Float,
            BaseType::Double,
            BaseType::Decimal,
            BaseType::Varchar,
            BaseType::Boolean,
            BaseType::Date,
            BaseType::Timestamp,
        ]
    }

    fn cast_sql(self) -> &'static str {
        match self {
            BaseType::SmallInt => "CAST(1 AS SMALLINT)",
            BaseType::Integer => "CAST(42 AS INTEGER)",
            BaseType::BigInt => "CAST(100 AS BIGINT)",
            BaseType::Float => "CAST(1.5 AS FLOAT)",
            BaseType::Double => "CAST(3.14 AS DOUBLE)",
            BaseType::Decimal => "CAST(99.99 AS DECIMAL(10,2))",
            BaseType::Varchar => "CAST('hello' AS VARCHAR)",
            BaseType::Boolean => "CAST(TRUE AS BOOLEAN)",
            BaseType::Date => "CAST('2024-01-01' AS DATE)",
            BaseType::Timestamp => "CAST('2024-01-01 12:00:00' AS TIMESTAMP)",
        }
    }
}

// ---- SQL generation helpers ----

/// Scalar subquery in SELECT position: SELECT (SELECT CAST(v AS T) AS x) AS result
fn scalar_subquery_select(inner_type: BaseType) -> String {
    format!("SELECT (SELECT {} AS x) AS result", inner_type.cast_sql())
}

/// Scalar subquery with CTE providing data:
/// WITH data AS (SELECT CAST(v AS T) AS x) SELECT (SELECT x FROM data) AS result
fn scalar_subquery_from_cte(inner_type: BaseType) -> String {
    format!(
        "WITH data AS (SELECT {} AS x) SELECT (SELECT x FROM data) AS result",
        inner_type.cast_sql()
    )
}

/// Scalar subquery with expression applied to subquery result:
/// SELECT (SELECT CAST(v AS T)) + 0 AS result (for numeric types)
fn scalar_subquery_with_arithmetic(inner_type: BaseType) -> String {
    format!(
        "SELECT (SELECT {} AS x) + 0 AS result",
        inner_type.cast_sql()
    )
}

/// Two scalar subqueries of different types:
/// SELECT (SELECT CAST(v1 AS T1)) AS c0, (SELECT CAST(v2 AS T2)) AS c1
fn two_scalar_subqueries(t1: BaseType, t2: BaseType) -> String {
    format!(
        "SELECT (SELECT {} AS x) AS c0, (SELECT {} AS y) AS c1",
        t1.cast_sql(),
        t2.cast_sql()
    )
}

/// IN subquery: WITH data AS (SELECT typed col) SELECT CAST(v AS T) IN (SELECT x FROM data) AS result
fn in_subquery_query(list_type: BaseType) -> String {
    format!(
        "WITH data AS (SELECT {} AS x) SELECT {} IN (SELECT x FROM data) AS result",
        list_type.cast_sql(),
        list_type.cast_sql()
    )
}

/// EXISTS subquery: WITH data AS (SELECT typed col) SELECT EXISTS (SELECT x FROM data) AS result
fn exists_subquery_query(inner_type: BaseType) -> String {
    format!(
        "WITH data AS (SELECT {} AS x) SELECT EXISTS (SELECT x FROM data) AS result",
        inner_type.cast_sql()
    )
}

/// Nested scalar subquery: SELECT (SELECT (SELECT CAST(v AS T))) AS result
fn nested_scalar_subquery(inner_type: BaseType) -> String {
    format!(
        "SELECT (SELECT (SELECT {} AS x) AS y) AS result",
        inner_type.cast_sql()
    )
}

/// Scalar subquery in WHERE with non-subquery SELECT:
/// WITH data AS (SELECT CAST(v AS T1) AS x, CAST(v2 AS T2) AS y)
/// SELECT y FROM data WHERE x IN (SELECT CAST(v AS T1))
fn select_with_in_subquery_filter(select_type: BaseType, filter_type: BaseType) -> String {
    format!(
        "WITH data AS (SELECT {} AS x, {} AS y) \
         SELECT y FROM data WHERE x IN (SELECT {} AS v)",
        filter_type.cast_sql(),
        select_type.cast_sql(),
        filter_type.cast_sql()
    )
}

/// Scalar subquery combined with EXISTS in same SELECT:
/// SELECT (SELECT CAST(v AS T)) AS c0, EXISTS (SELECT 1) AS c1
fn scalar_and_exists_combined(inner_type: BaseType) -> String {
    format!(
        "SELECT (SELECT {} AS x) AS c0, EXISTS (SELECT 1) AS c1",
        inner_type.cast_sql()
    )
}

// ---- Inference helpers ----

fn run_smelt_inference(sql: &str) -> Vec<DataType> {
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt in parsed SQL");

    let base_ctx = TypeContext::new();
    let ctx = build_subquery_context(&select_stmt, &base_ctx);
    let column_types = infer_select_column_types(&select_stmt, &ctx);

    column_types.into_iter().map(|tc| tc.data_type).collect()
}

fn check_against_oracle(
    oracle: &dyn TypeOracle,
    sql: &str,
    divergences: &[TypeDivergence],
) -> Result<(), String> {
    let actual_types = match oracle.query_types(sql) {
        Ok(types) => types,
        Err(e) => {
            return Err(format!("DuckDB rejected: {e}"));
        }
    };

    let inferred = run_smelt_inference(sql);

    for (i, actual) in actual_types.iter().enumerate() {
        let smelt_type = if i < inferred.len() {
            &inferred[i]
        } else {
            continue;
        };
        let actual_type = &actual.1;

        if *smelt_type == DataType::unknown_dynamic() {
            continue;
        }

        match compare_types(smelt_type, actual_type) {
            TypeMatch::Exact | TypeMatch::Compatible { .. } => {}
            TypeMatch::Mismatch => {
                if find_divergence(smelt_type, actual_type, "duckdb", divergences).is_none() {
                    return Err(format!(
                        "Subquery type mismatch for column {} ({}):\n  \
                         smelt inferred: {smelt_type:?}\n  \
                         duckdb actual:  {actual_type:?}\n  \
                         SQL: {sql}",
                        i, actual.0
                    ));
                }
            }
        }
    }
    Ok(())
}

// ---- Proptest strategies ----

fn base_type_strategy() -> impl Strategy<Value = BaseType> {
    prop_oneof![
        Just(BaseType::SmallInt),
        Just(BaseType::Integer),
        Just(BaseType::BigInt),
        Just(BaseType::Float),
        Just(BaseType::Double),
        Just(BaseType::Decimal),
        Just(BaseType::Varchar),
        Just(BaseType::Boolean),
        Just(BaseType::Date),
        Just(BaseType::Timestamp),
    ]
}

// ---- Proptests ----

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Scalar subqueries in SELECT position should match the inner type.
    #[test]
    fn prop_scalar_subquery_select_type(t in base_type_strategy()) {
        let sql = scalar_subquery_select(t);
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        match check_against_oracle(&duckdb, &sql, &divergences) {
            Ok(()) => {}
            Err(msg) => {
                if !msg.starts_with("DuckDB rejected") {
                    prop_assert!(false, "{}", msg);
                }
            }
        }
    }

    /// Scalar subqueries from CTE should infer correctly.
    #[test]
    fn prop_scalar_subquery_from_cte(t in base_type_strategy()) {
        let sql = scalar_subquery_from_cte(t);
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        match check_against_oracle(&duckdb, &sql, &divergences) {
            Ok(()) => {}
            Err(msg) => {
                if !msg.starts_with("DuckDB rejected") {
                    prop_assert!(false, "{}", msg);
                }
            }
        }
    }

    /// Two scalar subqueries of different types should each infer correctly.
    #[test]
    fn prop_two_scalar_subqueries(t1 in base_type_strategy(), t2 in base_type_strategy()) {
        let sql = two_scalar_subqueries(t1, t2);
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        match check_against_oracle(&duckdb, &sql, &divergences) {
            Ok(()) => {}
            Err(msg) => {
                if !msg.starts_with("DuckDB rejected") {
                    prop_assert!(false, "{}", msg);
                }
            }
        }
    }

    /// IN subqueries should always return Boolean.
    #[test]
    fn prop_in_subquery_type(t in base_type_strategy()) {
        let sql = in_subquery_query(t);
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        match check_against_oracle(&duckdb, &sql, &divergences) {
            Ok(()) => {}
            Err(msg) => {
                if !msg.starts_with("DuckDB rejected") {
                    prop_assert!(false, "{}", msg);
                }
            }
        }
    }
}

// ---- Exhaustive deterministic tests ----

/// Test scalar subquery type inference for all base types.
#[test]
fn exhaustive_scalar_subquery_all_types() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let mut failures = Vec::new();

    for ty in BaseType::all() {
        let sql = scalar_subquery_select(*ty);
        match check_against_oracle(&duckdb, &sql, &divergences) {
            Ok(()) => {}
            Err(msg) if msg.starts_with("DuckDB rejected") => {}
            Err(msg) => failures.push(msg),
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} scalar subquery failures:\n{}",
            failures.len(),
            failures.join("\n---\n")
        );
    }
}

/// Test EXISTS subquery for all inner types — should always be Boolean.
#[test]
fn exhaustive_exists_subquery_all_types() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let mut failures = Vec::new();

    for ty in BaseType::all() {
        let sql = exists_subquery_query(*ty);
        match check_against_oracle(&duckdb, &sql, &divergences) {
            Ok(()) => {}
            Err(msg) if msg.starts_with("DuckDB rejected") => {}
            Err(msg) => failures.push(msg),
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} EXISTS subquery failures:\n{}",
            failures.len(),
            failures.join("\n---\n")
        );
    }
}

/// Test two scalar subqueries across all type pairs (10×10 = 100 combinations).
#[test]
fn exhaustive_two_scalar_subqueries_all_pairs() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let mut failures = Vec::new();

    for t1 in BaseType::all() {
        for t2 in BaseType::all() {
            let sql = two_scalar_subqueries(*t1, *t2);
            match check_against_oracle(&duckdb, &sql, &divergences) {
                Ok(()) => {}
                Err(msg) if msg.starts_with("DuckDB rejected") => {}
                Err(msg) => failures.push(msg),
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} two-scalar-subquery failures:\n{}",
            failures.len(),
            failures.join("\n---\n")
        );
    }
}

/// Test SELECT with IN subquery filter — result type should be the select column type.
#[test]
fn exhaustive_in_subquery_filter_preserves_select_type() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let mut failures = Vec::new();

    for select_ty in BaseType::all() {
        for filter_ty in BaseType::all() {
            let sql = select_with_in_subquery_filter(*select_ty, *filter_ty);
            match check_against_oracle(&duckdb, &sql, &divergences) {
                Ok(()) => {}
                Err(msg) if msg.starts_with("DuckDB rejected") => {}
                Err(msg) => failures.push(msg),
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} IN subquery filter failures:\n{}",
            failures.len(),
            failures.join("\n---\n")
        );
    }
}

/// Test nested scalar subqueries for all base types.
#[test]
fn exhaustive_nested_scalar_subquery_all_types() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let mut failures = Vec::new();

    for ty in BaseType::all() {
        let sql = nested_scalar_subquery(*ty);
        match check_against_oracle(&duckdb, &sql, &divergences) {
            Ok(()) => {}
            Err(msg) if msg.starts_with("DuckDB rejected") => {}
            Err(msg) => failures.push(msg),
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} nested scalar subquery failures:\n{}",
            failures.len(),
            failures.join("\n---\n")
        );
    }
}

/// Test scalar subquery combined with EXISTS for all base types.
#[test]
fn exhaustive_scalar_and_exists_combined_all_types() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let mut failures = Vec::new();

    for ty in BaseType::all() {
        let sql = scalar_and_exists_combined(*ty);
        match check_against_oracle(&duckdb, &sql, &divergences) {
            Ok(()) => {}
            Err(msg) if msg.starts_with("DuckDB rejected") => {}
            Err(msg) => failures.push(msg),
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} scalar+EXISTS combined failures:\n{}",
            failures.len(),
            failures.join("\n---\n")
        );
    }
}

/// Test scalar subquery with arithmetic for all numeric types.
#[test]
fn exhaustive_scalar_subquery_with_arithmetic_numeric() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let mut failures = Vec::new();

    let numeric_types = [
        BaseType::SmallInt,
        BaseType::Integer,
        BaseType::BigInt,
        BaseType::Float,
        BaseType::Double,
        BaseType::Decimal,
    ];

    for ty in &numeric_types {
        let sql = scalar_subquery_with_arithmetic(*ty);
        match check_against_oracle(&duckdb, &sql, &divergences) {
            Ok(()) => {}
            Err(msg) if msg.starts_with("DuckDB rejected") => {}
            Err(msg) => failures.push(msg),
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} scalar subquery arithmetic failures:\n{}",
            failures.len(),
            failures.join("\n---\n")
        );
    }
}

// ---- Smoke tests ----

#[test]
fn smoke_scalar_subquery_integer() {
    let inferred = run_smelt_inference("SELECT (SELECT CAST(42 AS INTEGER) AS x) AS result");
    assert_eq!(inferred[0], DataType::Integer);
}

#[test]
fn smoke_scalar_subquery_varchar() {
    let inferred = run_smelt_inference("SELECT (SELECT CAST('hello' AS VARCHAR) AS x) AS result");
    assert!(
        matches!(inferred[0], DataType::Varchar { .. } | DataType::Text),
        "Expected Varchar or Text, got {:?}",
        inferred[0]
    );
}

#[test]
fn smoke_scalar_subquery_from_cte() {
    let inferred = run_smelt_inference(
        "WITH data AS (SELECT CAST(42 AS INTEGER) AS x) SELECT (SELECT x FROM data) AS result",
    );
    assert_eq!(inferred[0], DataType::Integer);
}

#[test]
fn smoke_nested_scalar_subquery() {
    let inferred =
        run_smelt_inference("SELECT (SELECT (SELECT CAST(42 AS INTEGER) AS x) AS y) AS result");
    assert_eq!(inferred[0], DataType::Integer);
}

#[test]
fn smoke_in_subquery_returns_boolean() {
    let inferred = run_smelt_inference(
        "WITH data AS (SELECT CAST(1 AS INTEGER) AS x) \
         SELECT CAST(1 AS INTEGER) IN (SELECT x FROM data) AS result",
    );
    assert_eq!(inferred[0], DataType::Boolean);
}

#[test]
fn smoke_exists_returns_boolean() {
    let inferred = run_smelt_inference(
        "WITH data AS (SELECT 1 AS x) SELECT EXISTS (SELECT x FROM data) AS result",
    );
    assert_eq!(inferred[0], DataType::Boolean);
}

#[test]
fn smoke_scalar_subquery_with_arithmetic() {
    let inferred = run_smelt_inference("SELECT (SELECT CAST(42 AS INTEGER) AS x) + 0 AS result");
    // Integer + Integer literal → Integer
    assert!(
        matches!(
            inferred[0],
            DataType::Integer | DataType::BigInt | DataType::SmallInt
        ),
        "Expected numeric type, got {:?}",
        inferred[0]
    );
}

#[test]
fn smoke_scalar_and_exists_combined() {
    let inferred = run_smelt_inference(
        "SELECT (SELECT CAST(42 AS INTEGER) AS x) AS c0, EXISTS (SELECT 1) AS c1",
    );
    assert_eq!(inferred[0], DataType::Integer);
    assert_eq!(inferred[1], DataType::Boolean);
}

#[test]
fn smoke_scalar_subquery_boolean() {
    let inferred = run_smelt_inference("SELECT (SELECT CAST(TRUE AS BOOLEAN) AS x) AS result");
    assert_eq!(inferred[0], DataType::Boolean);
}

#[test]
fn smoke_scalar_subquery_timestamp() {
    let inferred = run_smelt_inference(
        "SELECT (SELECT CAST('2024-01-01 12:00:00' AS TIMESTAMP) AS x) AS result",
    );
    assert!(
        matches!(inferred[0], DataType::Timestamp { .. }),
        "Expected Timestamp, got {:?}",
        inferred[0]
    );
}
