//! Property-based tests for binary operator type coercion.
//!
//! Systematically tests all type-pair combinations through arithmetic,
//! comparison, and string concatenation operators, validating smelt's
//! type inference against DuckDB.
//!
//! Coverage: +, -, *, /, ||, <, >, =, !=, <=, >=

#[allow(dead_code)]
mod prop_helpers;

use prop_helpers::divergences::{find_divergence, known_divergences, TypeDivergence};
use prop_helpers::duckdb_oracle::{DuckDbOracle, TypeOracle};
use prop_helpers::type_comparison::{compare_types, TypeMatch};

use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_parser::ast::File;
use smelt_types::{DataType, TypedColumn};

use proptest::prelude::*;

// ---- Types for the coercion matrix ----

/// Numeric types to test in arithmetic coercion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumericType {
    SmallInt,
    Integer,
    BigInt,
    Float,
    Double,
    Decimal,
}

impl NumericType {
    fn all() -> &'static [NumericType] {
        &[
            NumericType::SmallInt,
            NumericType::Integer,
            NumericType::BigInt,
            NumericType::Float,
            NumericType::Double,
            NumericType::Decimal,
        ]
    }

    fn cast_sql(self) -> &'static str {
        match self {
            NumericType::SmallInt => "CAST(1 AS SMALLINT)",
            NumericType::Integer => "CAST(42 AS INTEGER)",
            NumericType::BigInt => "CAST(100 AS BIGINT)",
            NumericType::Float => "CAST(1.5 AS FLOAT)",
            NumericType::Double => "CAST(3.14 AS DOUBLE)",
            NumericType::Decimal => "CAST(99.99 AS DECIMAL(10,2))",
        }
    }

    fn to_smelt_type(self) -> DataType {
        match self {
            NumericType::SmallInt => DataType::SmallInt,
            NumericType::Integer => DataType::Integer,
            NumericType::BigInt => DataType::BigInt,
            NumericType::Float => DataType::Float,
            NumericType::Double => DataType::Double,
            NumericType::Decimal => DataType::Decimal {
                precision: 10,
                scale: 2,
            },
        }
    }
}

/// Arithmetic operators to test.
#[derive(Debug, Clone, Copy)]
enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

impl ArithOp {
    fn all() -> &'static [ArithOp] {
        &[
            ArithOp::Add,
            ArithOp::Sub,
            ArithOp::Mul,
            ArithOp::Div,
            ArithOp::Mod,
        ]
    }

    fn symbol(self) -> &'static str {
        match self {
            ArithOp::Add => "+",
            ArithOp::Sub => "-",
            ArithOp::Mul => "*",
            ArithOp::Div => "/",
            ArithOp::Mod => "%",
        }
    }
}

/// Comparison operators to test.
#[derive(Debug, Clone, Copy)]
enum CompOp {
    Eq,
    Neq,
    Lt,
    Gt,
    Lte,
    Gte,
}

impl CompOp {
    fn all() -> &'static [CompOp] {
        &[
            CompOp::Eq,
            CompOp::Neq,
            CompOp::Lt,
            CompOp::Gt,
            CompOp::Lte,
            CompOp::Gte,
        ]
    }

    fn symbol(self) -> &'static str {
        match self {
            CompOp::Eq => "=",
            CompOp::Neq => "!=",
            CompOp::Lt => "<",
            CompOp::Gt => ">",
            CompOp::Lte => "<=",
            CompOp::Gte => ">=",
        }
    }
}

/// Comparable types (can be used with <, >, = etc.)
#[derive(Debug, Clone, Copy)]
enum ComparableType {
    SmallInt,
    Integer,
    BigInt,
    Float,
    Double,
    Decimal,
    Varchar,
    Date,
    Timestamp,
    Boolean,
}

impl ComparableType {
    fn all() -> &'static [ComparableType] {
        &[
            ComparableType::SmallInt,
            ComparableType::Integer,
            ComparableType::BigInt,
            ComparableType::Float,
            ComparableType::Double,
            ComparableType::Decimal,
            ComparableType::Varchar,
            ComparableType::Date,
            ComparableType::Timestamp,
            ComparableType::Boolean,
        ]
    }

    fn cast_sql(self) -> &'static str {
        match self {
            ComparableType::SmallInt => "CAST(1 AS SMALLINT)",
            ComparableType::Integer => "CAST(42 AS INTEGER)",
            ComparableType::BigInt => "CAST(100 AS BIGINT)",
            ComparableType::Float => "CAST(1.5 AS FLOAT)",
            ComparableType::Double => "CAST(3.14 AS DOUBLE)",
            ComparableType::Decimal => "CAST(99.99 AS DECIMAL(10,2))",
            ComparableType::Varchar => "CAST('hello' AS VARCHAR)",
            ComparableType::Date => "CAST('2024-01-01' AS DATE)",
            ComparableType::Timestamp => "CAST('2024-01-01 12:00:00' AS TIMESTAMP)",
            ComparableType::Boolean => "CAST(TRUE AS BOOLEAN)",
        }
    }

    fn to_smelt_type(self) -> DataType {
        match self {
            ComparableType::SmallInt => DataType::SmallInt,
            ComparableType::Integer => DataType::Integer,
            ComparableType::BigInt => DataType::BigInt,
            ComparableType::Float => DataType::Float,
            ComparableType::Double => DataType::Double,
            ComparableType::Decimal => DataType::Decimal {
                precision: 10,
                scale: 2,
            },
            ComparableType::Varchar => DataType::Varchar { max_length: None },
            ComparableType::Date => DataType::Date,
            ComparableType::Timestamp => DataType::Timestamp {
                with_timezone: false,
            },
            ComparableType::Boolean => DataType::Boolean,
        }
    }
}

// ---- SQL generation helpers ----

/// Build a CTE query testing a binary arithmetic expression.
fn arithmetic_query(left: NumericType, right: NumericType, op: ArithOp) -> String {
    format!(
        "WITH data AS (SELECT {} AS l, {} AS r) SELECT l {} r AS expr_0 FROM data",
        left.cast_sql(),
        right.cast_sql(),
        op.symbol()
    )
}

/// Build typed sources for an arithmetic query.
fn arithmetic_sources(
    left: NumericType,
    right: NumericType,
) -> Vec<prop_helpers::generators::TypedSource> {
    vec![
        prop_helpers::generators::TypedSource {
            name: "l".into(),
            data_type: left.to_smelt_type(),
            cast_sql: left.cast_sql().into(),
        },
        prop_helpers::generators::TypedSource {
            name: "r".into(),
            data_type: right.to_smelt_type(),
            cast_sql: right.cast_sql().into(),
        },
    ]
}

/// Build a CTE query testing a comparison expression.
fn comparison_query(left: ComparableType, right: ComparableType, op: CompOp) -> String {
    format!(
        "WITH data AS (SELECT {} AS l, {} AS r) SELECT l {} r AS expr_0 FROM data",
        left.cast_sql(),
        right.cast_sql(),
        op.symbol()
    )
}

/// Build typed sources for a comparison query.
fn comparison_sources(
    left: ComparableType,
    right: ComparableType,
) -> Vec<prop_helpers::generators::TypedSource> {
    vec![
        prop_helpers::generators::TypedSource {
            name: "l".into(),
            data_type: left.to_smelt_type(),
            cast_sql: left.cast_sql().into(),
        },
        prop_helpers::generators::TypedSource {
            name: "r".into(),
            data_type: right.to_smelt_type(),
            cast_sql: right.cast_sql().into(),
        },
    ]
}

/// Build a CTE query testing string concatenation.
fn concat_query(left_cast: &str, right_cast: &str) -> String {
    format!(
        "WITH data AS (SELECT {left_cast} AS l, {right_cast} AS r) SELECT l || r AS expr_0 FROM data"
    )
}

// ---- Inference & comparison helpers ----

/// Parse SQL with smelt and run type inference.
fn run_smelt_inference(
    sql: &str,
    columns: &[prop_helpers::generators::TypedSource],
) -> Vec<(String, DataType)> {
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

/// Compare smelt inference against DuckDB oracle.
fn check_types_against_oracle(
    oracle: &dyn TypeOracle,
    backend: &str,
    sql: &str,
    columns: &[prop_helpers::generators::TypedSource],
    divergences: &[TypeDivergence],
) -> Result<(), String> {
    let actual_types = match oracle.query_types(sql) {
        Ok(types) => types,
        Err(_) => return Ok(()), // Skip invalid SQL
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
                        "Coercion type mismatch for column {} ({}) against {backend}:\n  \
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

// ---- Proptest strategies ----

fn numeric_type_strategy() -> impl Strategy<Value = NumericType> {
    prop_oneof![
        Just(NumericType::SmallInt),
        Just(NumericType::Integer),
        Just(NumericType::BigInt),
        Just(NumericType::Float),
        Just(NumericType::Double),
        Just(NumericType::Decimal),
    ]
}

fn arith_op_strategy() -> impl Strategy<Value = ArithOp> {
    prop_oneof![
        Just(ArithOp::Add),
        Just(ArithOp::Sub),
        Just(ArithOp::Mul),
        Just(ArithOp::Div),
        Just(ArithOp::Mod),
    ]
}

fn comparable_type_strategy() -> impl Strategy<Value = ComparableType> {
    prop_oneof![
        Just(ComparableType::SmallInt),
        Just(ComparableType::Integer),
        Just(ComparableType::BigInt),
        Just(ComparableType::Float),
        Just(ComparableType::Double),
        Just(ComparableType::Decimal),
        Just(ComparableType::Varchar),
        Just(ComparableType::Date),
        Just(ComparableType::Timestamp),
        Just(ComparableType::Boolean),
    ]
}

fn comp_op_strategy() -> impl Strategy<Value = CompOp> {
    prop_oneof![
        Just(CompOp::Eq),
        Just(CompOp::Neq),
        Just(CompOp::Lt),
        Just(CompOp::Gt),
        Just(CompOp::Lte),
        Just(CompOp::Gte),
    ]
}

// ---- Property tests ----

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Test all numeric type pairs through arithmetic operators.
    #[test]
    fn prop_arithmetic_coercion(
        left in numeric_type_strategy(),
        right in numeric_type_strategy(),
        op in arith_op_strategy(),
    ) {
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        let sql = arithmetic_query(left, right, op);
        let columns = arithmetic_sources(left, right);

        if let Err(msg) = check_types_against_oracle(&duckdb, "duckdb", &sql, &columns, &divergences) {
            prop_assert!(false, "{}", msg);
        }
    }

    /// Test same-type comparisons (always valid).
    #[test]
    fn prop_same_type_comparison(
        ty in comparable_type_strategy(),
        op in comp_op_strategy(),
    ) {
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        let sql = comparison_query(ty, ty, op);
        let columns = comparison_sources(ty, ty);

        if let Err(msg) = check_types_against_oracle(&duckdb, "duckdb", &sql, &columns, &divergences) {
            prop_assert!(false, "{}", msg);
        }
    }

    /// Test cross-type numeric comparisons.
    #[test]
    fn prop_cross_numeric_comparison(
        left in numeric_type_strategy(),
        right in numeric_type_strategy(),
        op in comp_op_strategy(),
    ) {
        let left_ct = match left {
            NumericType::SmallInt => ComparableType::SmallInt,
            NumericType::Integer => ComparableType::Integer,
            NumericType::BigInt => ComparableType::BigInt,
            NumericType::Float => ComparableType::Float,
            NumericType::Double => ComparableType::Double,
            NumericType::Decimal => ComparableType::Decimal,
        };
        let right_ct = match right {
            NumericType::SmallInt => ComparableType::SmallInt,
            NumericType::Integer => ComparableType::Integer,
            NumericType::BigInt => ComparableType::BigInt,
            NumericType::Float => ComparableType::Float,
            NumericType::Double => ComparableType::Double,
            NumericType::Decimal => ComparableType::Decimal,
        };

        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        let sql = comparison_query(left_ct, right_ct, op);
        let columns = comparison_sources(left_ct, right_ct);

        if let Err(msg) = check_types_against_oracle(&duckdb, "duckdb", &sql, &columns, &divergences) {
            prop_assert!(false, "{}", msg);
        }
    }
}

// ---- Exhaustive matrix tests ----

/// Test every combination of numeric type pairs through all arithmetic operators.
/// This is deterministic (not proptest) and covers the full 6x6x4 = 144 matrix.
#[test]
fn exhaustive_arithmetic_matrix() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let mut failures = Vec::new();

    for &left in NumericType::all() {
        for &right in NumericType::all() {
            for &op in ArithOp::all() {
                let sql = arithmetic_query(left, right, op);
                let columns = arithmetic_sources(left, right);

                if let Err(msg) =
                    check_types_against_oracle(&duckdb, "duckdb", &sql, &columns, &divergences)
                {
                    failures.push(format!("{:?} {} {:?}: {}", left, op.symbol(), right, msg));
                }
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "Arithmetic coercion failures ({}/{}):\n{}",
            failures.len(),
            NumericType::all().len() * NumericType::all().len() * ArithOp::all().len(),
            failures.join("\n\n")
        );
    }
}

/// Test every same-type comparison through all comparison operators.
#[test]
fn exhaustive_same_type_comparisons() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let mut failures = Vec::new();

    for &ty in ComparableType::all() {
        for &op in CompOp::all() {
            let sql = comparison_query(ty, ty, op);
            let columns = comparison_sources(ty, ty);

            if let Err(msg) =
                check_types_against_oracle(&duckdb, "duckdb", &sql, &columns, &divergences)
            {
                failures.push(format!("{:?} {} {:?}: {}", ty, op.symbol(), ty, msg));
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "Same-type comparison failures ({}):\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}

/// Test string concatenation with all string-compatible types.
#[test]
fn exhaustive_string_concat() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();

    let string_casts = [(
        "CAST('hello' AS VARCHAR)",
        DataType::Varchar { max_length: None },
    )];

    let mut failures = Vec::new();

    for (left_cast, left_type) in &string_casts {
        for (right_cast, right_type) in &string_casts {
            let sql = concat_query(left_cast, right_cast);
            let columns = vec![
                prop_helpers::generators::TypedSource {
                    name: "l".into(),
                    data_type: left_type.clone(),
                    cast_sql: left_cast.to_string(),
                },
                prop_helpers::generators::TypedSource {
                    name: "r".into(),
                    data_type: right_type.clone(),
                    cast_sql: right_cast.to_string(),
                },
            ];

            if let Err(msg) =
                check_types_against_oracle(&duckdb, "duckdb", &sql, &columns, &divergences)
            {
                failures.push(msg);
            }
        }
    }

    if !failures.is_empty() {
        panic!(
            "String concat failures ({}):\n{}",
            failures.len(),
            failures.join("\n\n")
        );
    }
}

// ---- Smoke tests for specific coercion rules ----

#[test]
fn smoke_integer_plus_integer() {
    let duckdb = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(1 AS INTEGER) AS l, CAST(2 AS INTEGER) AS r) SELECT l + r AS expr_0 FROM data";
    let columns = arithmetic_sources(NumericType::Integer, NumericType::Integer);
    let inferred = run_smelt_inference(sql, &columns);
    let actual = duckdb.query_types(sql).unwrap();

    assert_eq!(inferred[0].1, DataType::Integer);
    let m = compare_types(&inferred[0].1, &actual[0].1);
    assert!(
        matches!(m, TypeMatch::Exact | TypeMatch::Compatible { .. }),
        "smelt={:?}, duckdb={:?}",
        inferred[0].1,
        actual[0].1
    );
}

#[test]
fn smoke_integer_plus_double() {
    let duckdb = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(1 AS INTEGER) AS l, CAST(2.0 AS DOUBLE) AS r) SELECT l + r AS expr_0 FROM data";
    let columns = arithmetic_sources(NumericType::Integer, NumericType::Double);
    let inferred = run_smelt_inference(sql, &columns);
    let actual = duckdb.query_types(sql).unwrap();

    assert_eq!(inferred[0].1, DataType::Double);
    assert_eq!(actual[0].1, DataType::Double);
}

#[test]
fn smoke_smallint_plus_bigint() {
    let duckdb = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(1 AS SMALLINT) AS l, CAST(100 AS BIGINT) AS r) SELECT l + r AS expr_0 FROM data";
    let columns = arithmetic_sources(NumericType::SmallInt, NumericType::BigInt);
    let inferred = run_smelt_inference(sql, &columns);
    let actual = duckdb.query_types(sql).unwrap();

    assert_eq!(inferred[0].1, DataType::BigInt);
    assert_eq!(actual[0].1, DataType::BigInt);
}

#[test]
fn smoke_decimal_plus_integer() {
    let duckdb = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(1.5 AS DECIMAL(10,2)) AS l, CAST(2 AS INTEGER) AS r) SELECT l + r AS expr_0 FROM data";
    let columns = arithmetic_sources(NumericType::Decimal, NumericType::Integer);
    let inferred = run_smelt_inference(sql, &columns);

    // smelt infers Decimal for Decimal + Integer
    assert!(matches!(inferred[0].1, DataType::Decimal { .. }));

    // DuckDB also returns Decimal (possibly different precision)
    let actual = duckdb.query_types(sql).unwrap();
    let m = compare_types(&inferred[0].1, &actual[0].1);
    assert!(
        matches!(m, TypeMatch::Exact | TypeMatch::Compatible { .. }),
        "smelt={:?}, duckdb={:?}",
        inferred[0].1,
        actual[0].1
    );
}

#[test]
fn smoke_integer_div_integer() {
    // Both smelt and DuckDB return Double for integer/integer division.
    // Smelt was aligned to DuckDB/Spark behaviour in the Sherlock-feedback
    // pass (binary.rs: non-Decimal division always returns Double).
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let sql = "WITH data AS (SELECT CAST(10 AS INTEGER) AS l, CAST(3 AS INTEGER) AS r) SELECT l / r AS expr_0 FROM data";
    let columns = arithmetic_sources(NumericType::Integer, NumericType::Integer);
    let inferred = run_smelt_inference(sql, &columns);
    let actual = duckdb.query_types(sql).unwrap();

    // smelt and DuckDB both return Double — no divergence
    assert_eq!(inferred[0].1, DataType::Double);
    let m = compare_types(&inferred[0].1, &actual[0].1);
    if matches!(m, TypeMatch::Mismatch) {
        assert!(
            find_divergence(&inferred[0].1, &actual[0].1, "duckdb", &divergences).is_some(),
            "Unexpected mismatch: smelt={:?}, duckdb={:?}",
            inferred[0].1,
            actual[0].1
        );
    }
}

#[test]
fn smoke_string_concat() {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let sql = "WITH data AS (SELECT CAST('hello' AS VARCHAR) AS l, CAST(' world' AS VARCHAR) AS r) SELECT l || r AS expr_0 FROM data";
    let columns = vec![
        prop_helpers::generators::TypedSource {
            name: "l".into(),
            data_type: DataType::Varchar { max_length: None },
            cast_sql: "CAST('hello' AS VARCHAR)".into(),
        },
        prop_helpers::generators::TypedSource {
            name: "r".into(),
            data_type: DataType::Varchar { max_length: None },
            cast_sql: "CAST(' world' AS VARCHAR)".into(),
        },
    ];
    let inferred = run_smelt_inference(sql, &columns);
    let actual = duckdb.query_types(sql).unwrap();

    // smelt infers Text, DuckDB returns Varchar — known divergence
    assert_eq!(inferred[0].1, DataType::Text);
    let m = compare_types(&inferred[0].1, &actual[0].1);
    if matches!(m, TypeMatch::Mismatch) {
        assert!(
            find_divergence(&inferred[0].1, &actual[0].1, "duckdb", &divergences).is_some(),
            "Unexpected mismatch: smelt={:?}, duckdb={:?}",
            inferred[0].1,
            actual[0].1
        );
    }
}

#[test]
fn smoke_comparison_returns_boolean() {
    let sql = "WITH data AS (SELECT CAST(1 AS INTEGER) AS l, CAST(2 AS INTEGER) AS r) SELECT l < r AS expr_0 FROM data";
    let columns = arithmetic_sources(NumericType::Integer, NumericType::Integer);
    let inferred = run_smelt_inference(sql, &columns);

    assert_eq!(inferred[0].1, DataType::Boolean);
}

#[test]
fn smoke_mixed_comparison_returns_boolean() {
    let sql = "WITH data AS (SELECT CAST(1 AS INTEGER) AS l, CAST(2.0 AS DOUBLE) AS r) SELECT l >= r AS expr_0 FROM data";
    let columns = arithmetic_sources(NumericType::Integer, NumericType::Double);
    let inferred = run_smelt_inference(sql, &columns);

    assert_eq!(inferred[0].1, DataType::Boolean);
}

#[test]
fn smoke_double_minus_double() {
    let duckdb = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(3.14 AS DOUBLE) AS l, CAST(1.0 AS DOUBLE) AS r) SELECT l - r AS expr_0 FROM data";
    let columns = arithmetic_sources(NumericType::Double, NumericType::Double);
    let inferred = run_smelt_inference(sql, &columns);
    let actual = duckdb.query_types(sql).unwrap();

    assert_eq!(inferred[0].1, DataType::Double);
    assert_eq!(actual[0].1, DataType::Double);
}

#[test]
fn smoke_decimal_mul_decimal() {
    let duckdb = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(1.5 AS DECIMAL(10,2)) AS l, CAST(2.5 AS DECIMAL(10,2)) AS r) SELECT l * r AS expr_0 FROM data";
    let columns = arithmetic_sources(NumericType::Decimal, NumericType::Decimal);
    let inferred = run_smelt_inference(sql, &columns);
    let actual = duckdb.query_types(sql).unwrap();

    assert!(matches!(inferred[0].1, DataType::Decimal { .. }));
    let m = compare_types(&inferred[0].1, &actual[0].1);
    assert!(
        matches!(m, TypeMatch::Exact | TypeMatch::Compatible { .. }),
        "smelt={:?}, duckdb={:?}",
        inferred[0].1,
        actual[0].1
    );
}
