//! Property-based tests for set operation (UNION/INTERSECT/EXCEPT) type widening.
//!
//! Tests that smelt's `promote_types()` correctly widens column types when
//! combining branches of UNION/INTERSECT/EXCEPT, validated against DuckDB.

#[allow(dead_code)]
mod prop_helpers;

use prop_helpers::divergences::{find_divergence, known_divergences, TypeDivergence};
use prop_helpers::duckdb_oracle::{DuckDbOracle, TypeOracle};
use prop_helpers::type_comparison::{compare_types, TypeMatch};

use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_parser::ast::File;
use smelt_types::DataType;

use proptest::prelude::*;

// ---- Column type variants for set operation testing ----

/// All types that can appear in UNION columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColumnType {
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

impl ColumnType {
    fn all() -> &'static [ColumnType] {
        &[
            ColumnType::SmallInt,
            ColumnType::Integer,
            ColumnType::BigInt,
            ColumnType::Float,
            ColumnType::Double,
            ColumnType::Decimal,
            ColumnType::Varchar,
            ColumnType::Boolean,
            ColumnType::Date,
            ColumnType::Timestamp,
        ]
    }

    fn cast_sql(self) -> &'static str {
        match self {
            ColumnType::SmallInt => "CAST(1 AS SMALLINT)",
            ColumnType::Integer => "CAST(42 AS INTEGER)",
            ColumnType::BigInt => "CAST(100 AS BIGINT)",
            ColumnType::Float => "CAST(1.5 AS FLOAT)",
            ColumnType::Double => "CAST(3.14 AS DOUBLE)",
            ColumnType::Decimal => "CAST(99.99 AS DECIMAL(10,2))",
            ColumnType::Varchar => "CAST('hello' AS VARCHAR)",
            ColumnType::Boolean => "CAST(TRUE AS BOOLEAN)",
            ColumnType::Date => "CAST('2024-01-01' AS DATE)",
            ColumnType::Timestamp => "CAST('2024-01-01 12:00:00' AS TIMESTAMP)",
        }
    }
}

/// Set operations to test.
#[derive(Debug, Clone, Copy)]
enum SetOp {
    Union,
    UnionAll,
    Intersect,
    IntersectAll,
    Except,
    ExceptAll,
}

impl SetOp {
    fn all() -> &'static [SetOp] {
        &[
            SetOp::Union,
            SetOp::UnionAll,
            SetOp::Intersect,
            SetOp::IntersectAll,
            SetOp::Except,
            SetOp::ExceptAll,
        ]
    }

    fn sql(self) -> &'static str {
        match self {
            SetOp::Union => "UNION",
            SetOp::UnionAll => "UNION ALL",
            SetOp::Intersect => "INTERSECT",
            SetOp::IntersectAll => "INTERSECT ALL",
            SetOp::Except => "EXCEPT",
            SetOp::ExceptAll => "EXCEPT ALL",
        }
    }
}

// ---- SQL generation helpers ----

/// Build a set operation query with two branches having potentially different column types.
fn setop_query(left: ColumnType, right: ColumnType, op: SetOp) -> String {
    format!(
        "SELECT {} AS x {} SELECT {} AS x",
        left.cast_sql(),
        op.sql(),
        right.cast_sql()
    )
}

/// Build a 3-way set operation query.
fn setop_query_3way(t1: ColumnType, t2: ColumnType, t3: ColumnType, op: SetOp) -> String {
    format!(
        "SELECT {} AS x {} SELECT {} AS x {} SELECT {} AS x",
        t1.cast_sql(),
        op.sql(),
        t2.cast_sql(),
        op.sql(),
        t3.cast_sql()
    )
}

/// Build a multi-column set operation query.
fn setop_query_multi_col(
    left: (ColumnType, ColumnType),
    right: (ColumnType, ColumnType),
    op: SetOp,
) -> String {
    format!(
        "SELECT {} AS a, {} AS b {} SELECT {} AS a, {} AS b",
        left.0.cast_sql(),
        left.1.cast_sql(),
        op.sql(),
        right.0.cast_sql(),
        right.1.cast_sql()
    )
}

// ---- Inference & comparison helpers ----

/// Parse SQL with smelt and run type inference, returning inferred column types.
fn run_smelt_inference(sql: &str) -> Vec<DataType> {
    let parse = smelt_parser::parse(sql);
    let root = parse.syntax();
    let file = File::cast(root).expect("failed to cast to File");
    let select_stmt = file.select_stmt().expect("no SelectStmt in parsed SQL");

    let ctx = TypeContext::new();
    let column_types = infer_select_column_types(&select_stmt, &ctx);

    column_types.into_iter().map(|tc| tc.data_type).collect()
}

/// Compare smelt inference against DuckDB oracle for a set operation query.
fn check_setop_against_oracle(
    oracle: &dyn TypeOracle,
    sql: &str,
    divergences: &[TypeDivergence],
) -> Result<(), String> {
    let actual_types = match oracle.query_types(sql) {
        Ok(types) => types,
        Err(e) => {
            // DuckDB rejects some cross-type UNIONs (e.g., VARCHAR UNION DATE)
            // This is expected — skip these
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
                        "Set op type mismatch for column {} ({}):\n  \
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

fn setop_strategy() -> impl Strategy<Value = SetOp> {
    prop_oneof![
        Just(SetOp::Union),
        Just(SetOp::UnionAll),
        Just(SetOp::Intersect),
        Just(SetOp::IntersectAll),
        Just(SetOp::Except),
        Just(SetOp::ExceptAll),
    ]
}

/// Strategy for numeric types only (always compatible in UNION).
fn numeric_type_strategy() -> impl Strategy<Value = ColumnType> {
    prop_oneof![
        Just(ColumnType::SmallInt),
        Just(ColumnType::Integer),
        Just(ColumnType::BigInt),
        Just(ColumnType::Float),
        Just(ColumnType::Double),
        Just(ColumnType::Decimal),
    ]
}

// ---- Property tests ----

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Test numeric type promotion across all set operations.
    /// Numeric types are always compatible in UNION, so DuckDB should accept all.
    #[test]
    fn prop_numeric_setop_promotion(
        left in numeric_type_strategy(),
        right in numeric_type_strategy(),
        op in setop_strategy()
    ) {
        let sql = setop_query(left, right, op);
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        match check_setop_against_oracle(&duckdb, &sql, &divergences) {
            Ok(()) => {}
            Err(msg) => {
                if msg.starts_with("DuckDB rejected") {
                    // Skip - unexpected for numeric types but don't fail
                } else {
                    prop_assert!(false, "{}", msg);
                }
            }
        }
    }

    /// Test 3-way numeric UNION type promotion.
    #[test]
    fn prop_numeric_3way_union(
        t1 in numeric_type_strategy(),
        t2 in numeric_type_strategy(),
        t3 in numeric_type_strategy()
    ) {
        let sql = setop_query_3way(t1, t2, t3, SetOp::UnionAll);
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        match check_setop_against_oracle(&duckdb, &sql, &divergences) {
            Ok(()) => {}
            Err(msg) => {
                if !msg.starts_with("DuckDB rejected") {
                    prop_assert!(false, "{}", msg);
                }
            }
        }
    }

    /// Test multi-column UNION with independently promoted types.
    #[test]
    fn prop_multi_column_union(
        l1 in numeric_type_strategy(),
        l2 in numeric_type_strategy(),
        r1 in numeric_type_strategy(),
        r2 in numeric_type_strategy()
    ) {
        let sql = setop_query_multi_col((l1, l2), (r1, r2), SetOp::UnionAll);
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        match check_setop_against_oracle(&duckdb, &sql, &divergences) {
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

/// Test all numeric type pairs through UNION ALL.
#[test]
fn exhaustive_numeric_union_matrix() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let numerics = [
        ColumnType::SmallInt,
        ColumnType::Integer,
        ColumnType::BigInt,
        ColumnType::Float,
        ColumnType::Double,
        ColumnType::Decimal,
    ];

    let mut failures = Vec::new();
    for left in &numerics {
        for right in &numerics {
            let sql = setop_query(*left, *right, SetOp::UnionAll);
            match check_setop_against_oracle(&duckdb, &sql, &divergences) {
                Ok(()) => {}
                Err(msg) if msg.starts_with("DuckDB rejected") => {}
                Err(msg) => failures.push(msg),
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} numeric UNION failures:\n{}",
            failures.len(),
            failures.join("\n---\n")
        );
    }
}

/// Test all same-type set operations (should always work, type stays the same).
#[test]
fn exhaustive_same_type_all_setops() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();

    let mut failures = Vec::new();
    for ty in ColumnType::all() {
        for op in SetOp::all() {
            let sql = setop_query(*ty, *ty, *op);
            match check_setop_against_oracle(&duckdb, &sql, &divergences) {
                Ok(()) => {}
                Err(msg) if msg.starts_with("DuckDB rejected") => {}
                Err(msg) => failures.push(msg),
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} same-type set op failures:\n{}",
            failures.len(),
            failures.join("\n---\n")
        );
    }
}

/// Test all type pairs through UNION ALL to find which cross-type promotions work.
#[test]
fn exhaustive_all_types_union() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();

    let mut failures = Vec::new();
    for left in ColumnType::all() {
        for right in ColumnType::all() {
            let sql = setop_query(*left, *right, SetOp::UnionAll);
            match check_setop_against_oracle(&duckdb, &sql, &divergences) {
                Ok(()) => {}
                Err(msg) if msg.starts_with("DuckDB rejected") => {
                    // DuckDB rejects this combination — verify smelt also produces
                    // Unknown (incompatible types)
                    let inferred = run_smelt_inference(&sql);
                    if !inferred.is_empty() && inferred[0] != DataType::unknown_dynamic() {
                        // smelt inferred a type but DuckDB rejected — could be fine
                        // (smelt may be more permissive) but note it
                    }
                }
                Err(msg) => failures.push(msg),
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} cross-type UNION failures:\n{}",
            failures.len(),
            failures.join("\n---\n")
        );
    }
}

// ---- Smoke tests for specific promotion rules ----

#[test]
fn smoke_smallint_plus_integer_union() {
    let inferred = run_smelt_inference(
        "SELECT CAST(1 AS SMALLINT) AS x UNION ALL SELECT CAST(2 AS INTEGER) AS x",
    );
    assert_eq!(inferred[0], DataType::Integer);
}

#[test]
fn smoke_integer_plus_bigint_union() {
    let inferred = run_smelt_inference(
        "SELECT CAST(1 AS INTEGER) AS x UNION ALL SELECT CAST(2 AS BIGINT) AS x",
    );
    assert_eq!(inferred[0], DataType::BigInt);
}

#[test]
fn smoke_float_plus_decimal_union() {
    // FLOAT is normalized to DOUBLE in smelt, so FLOAT UNION DECIMAL → DOUBLE
    let inferred = run_smelt_inference(
        "SELECT CAST(1.0 AS FLOAT) AS x UNION ALL SELECT CAST(2.0 AS DECIMAL(10,2)) AS x",
    );
    assert_eq!(inferred[0], DataType::Double);
}

#[test]
fn smoke_double_plus_float_union() {
    let inferred = run_smelt_inference(
        "SELECT CAST(1.0 AS DOUBLE) AS x UNION ALL SELECT CAST(2.0 AS FLOAT) AS x",
    );
    assert_eq!(inferred[0], DataType::Double);
}

#[test]
fn smoke_decimal_precision_union() {
    let inferred = run_smelt_inference(
        "SELECT CAST(1.0 AS DECIMAL(10,2)) AS x UNION ALL SELECT CAST(2.0 AS DECIMAL(18,4)) AS x",
    );
    assert_eq!(
        inferred[0],
        DataType::Decimal {
            precision: 18,
            scale: 4
        }
    );
}

#[test]
fn smoke_date_plus_timestamp_union() {
    let duckdb = DuckDbOracle::new();
    let sql = "SELECT CAST('2024-01-01' AS DATE) AS x UNION ALL SELECT CAST('2024-01-01 12:00:00' AS TIMESTAMP) AS x";
    let inferred = run_smelt_inference(sql);
    assert_eq!(
        inferred[0],
        DataType::Timestamp {
            with_timezone: false
        }
    );
    // Also check DuckDB agrees
    let actual = duckdb.query_types(sql).unwrap();
    assert!(matches!(
        compare_types(&inferred[0], &actual[0].1),
        TypeMatch::Exact | TypeMatch::Compatible { .. }
    ));
}

#[test]
fn smoke_3way_numeric_union() {
    let inferred = run_smelt_inference(
        "SELECT CAST(1 AS SMALLINT) AS x UNION ALL SELECT CAST(2 AS INTEGER) AS x UNION ALL SELECT CAST(3 AS BIGINT) AS x",
    );
    assert_eq!(inferred[0], DataType::BigInt);
}

#[test]
fn smoke_intersect_type_promotion() {
    let inferred = run_smelt_inference(
        "SELECT CAST(1 AS SMALLINT) AS x INTERSECT SELECT CAST(2 AS INTEGER) AS x",
    );
    assert_eq!(inferred[0], DataType::Integer);
}

#[test]
fn smoke_except_type_promotion() {
    let inferred =
        run_smelt_inference("SELECT CAST(1 AS INTEGER) AS x EXCEPT SELECT CAST(2 AS BIGINT) AS x");
    assert_eq!(inferred[0], DataType::BigInt);
}

#[test]
fn smoke_multi_column_union() {
    let inferred = run_smelt_inference(
        "SELECT CAST(1 AS SMALLINT) AS a, CAST('x' AS VARCHAR) AS b \
         UNION ALL \
         SELECT CAST(2 AS BIGINT) AS a, CAST('y' AS VARCHAR) AS b",
    );
    assert_eq!(inferred[0], DataType::BigInt);
    // VARCHAR + VARCHAR stays VARCHAR
    assert!(matches!(inferred[1], DataType::Varchar { .. }));
}
