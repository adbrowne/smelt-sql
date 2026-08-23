//! Targeted, deterministic type property test over numeric functions.
//!
//! `type_property_tests.rs`'s general fuzzer picks one function per case from a
//! pool of hundreds, so hitting e.g. `MEDIAN` specifically on a `Decimal`
//! column is a low-probability draw — several real divergences (decimal
//! growth on `IFNULL`/`ROUND`, `MEDIAN(DECIMAL)`, `CAST(x AS FLOAT)`) only
//! surfaced after soaking that fuzzer to thousands of cases. This test instead
//! runs a dense, deterministic cross-product — every numeric function this
//! file knows about × every numeric `DataType` variant (and, for two-arg
//! functions, every pairing of two variants) — against DuckDB (always) and
//! Spark (when available), the same way `type_property_tests.rs` does.
//!
//! Unlike proptest's shrink-to-first-failure model, this test accumulates
//! every mismatch across the whole grid and reports them together: a
//! deterministic grid benefits from a full report, not a single counterexample.

#[allow(dead_code)]
mod prop_helpers;

use prop_helpers::divergences::known_divergences;
use prop_helpers::generators::{TypedExpr, TypedSource};
use prop_helpers::known_unknowns::known_unknowns;
use prop_helpers::oracle_check::check_types_against_oracle;
use smelt_oracle_testkit::DuckDbOracle;
use smelt_oracle_testkit::SparkOracle;

use smelt_types::DataType;

use std::sync::LazyLock;

/// Shared SparkOracle instance — created once on first access, same as
/// `type_property_tests.rs`.
static SPARK: LazyLock<Option<SparkOracle>> = LazyLock::new(|| {
    std::env::var("SPARK_CONTAINER_ID")
        .ok()
        .map(|id| SparkOracle::new(&id))
});

/// The numeric type matrix. Covers every `DataType::is_numeric()` variant
/// (`SmallInt`/`Integer`/`BigInt`/`Float`/`Double`/`Decimal`), plus two
/// distinct `Decimal` shapes (a typical fractional shape and a scale-0
/// integer-like shape) since several found divergences were precision-growth
/// bugs that only show up on `Decimal` operands.
fn numeric_types() -> Vec<(&'static str, DataType, &'static str)> {
    vec![
        ("smallint", DataType::SmallInt, "CAST(3 AS SMALLINT)"),
        ("integer", DataType::Integer, "CAST(42 AS INTEGER)"),
        ("bigint", DataType::BigInt, "CAST(100 AS BIGINT)"),
        ("float", DataType::Float, "CAST(3.14 AS FLOAT)"),
        ("double", DataType::Double, "CAST(3.14 AS DOUBLE)"),
        (
            "decimal_10_2",
            DataType::Decimal {
                precision: 10,
                scale: 2,
            },
            "CAST(99.99 AS DECIMAL(10,2))",
        ),
        (
            "decimal_5_0",
            DataType::Decimal {
                precision: 5,
                scale: 0,
            },
            "CAST(12345 AS DECIMAL(5,0))",
        ),
    ]
}

/// Single-argument numeric scalar functions: `FUNC(x)`.
fn single_arg_scalar_functions() -> &'static [&'static str] {
    &[
        "ABS", "SIGN", "CEIL", "FLOOR", "ROUND", "SQRT", "LN", "EXP", "LOG",
    ]
}

/// Single-argument numeric aggregate functions: `FUNC(x)` over one row of data.
fn single_arg_aggregate_functions() -> &'static [&'static str] {
    &[
        "SUM",
        "AVG",
        "MEDIAN",
        "MIN",
        "MAX",
        "STDDEV",
        "STDDEV_POP",
        "VARIANCE",
        "VAR_POP",
    ]
}

/// Two-argument numeric functions: `FUNC(a, b)`, tested with every ordered
/// pairing of numeric types (`a` and `b` independently drawn from the
/// matrix) — this is exactly the shape that caught decimal-widening bugs in
/// `IFNULL`/`GREATEST`/`LEAST` (spec §15's `decimal_arithmetic_model` class).
fn two_arg_functions() -> &'static [&'static str] {
    &[
        "MOD", "POWER", "GREATEST", "LEAST", "IFNULL", "COALESCE", "NULLIF",
    ]
}

/// Run one (function, columns) case against DuckDB and Spark, appending any
/// failure message to `failures` rather than stopping at the first one — a
/// deterministic grid should report everything wrong in one pass.
fn run_case(
    sql: &str,
    expr_sql: &str,
    columns: &[TypedSource],
    label: &str,
    failures: &mut Vec<String>,
) {
    let duckdb = DuckDbOracle::new();
    let divergences = known_divergences();
    let unknowns = known_unknowns();
    let exprs = vec![TypedExpr {
        sql: expr_sql.to_string(),
        alias: "expr_0".to_string(),
        expected_smelt_type: DataType::unknown_dynamic(),
    }];

    if let Err(msg) = check_types_against_oracle(
        &duckdb,
        "duckdb",
        sql,
        columns,
        &exprs,
        &divergences,
        &unknowns,
    ) {
        failures.push(format!("[{label}] {msg}"));
    }

    if let Some(spark) = SPARK.as_ref() {
        if let Err(msg) = check_types_against_oracle(
            spark,
            "spark",
            sql,
            columns,
            &exprs,
            &divergences,
            &unknowns,
        ) {
            failures.push(format!("[{label}] {msg}"));
        }
    }
}

#[test]
fn prop_numeric_single_arg_functions() {
    let mut failures = Vec::new();
    let types = numeric_types();

    for func in single_arg_scalar_functions()
        .iter()
        .chain(single_arg_aggregate_functions())
    {
        for (type_label, data_type, cast_sql) in &types {
            let columns = vec![TypedSource {
                name: "x".to_string(),
                data_type: data_type.clone(),
                cast_sql: cast_sql.to_string(),
            }];
            let expr_sql = format!("{func}(x)");
            let sql = format!(
                "WITH data AS (SELECT {cast_sql} AS x) SELECT {expr_sql} AS expr_0 FROM data"
            );
            run_case(
                &sql,
                &expr_sql,
                &columns,
                &format!("{func}({type_label})"),
                &mut failures,
            );
        }
    }

    assert!(
        failures.is_empty(),
        "{} numeric single-arg function type mismatch(es):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn prop_numeric_two_arg_functions() {
    let mut failures = Vec::new();
    let types = numeric_types();

    for func in two_arg_functions() {
        for (a_label, a_type, a_cast) in &types {
            for (b_label, b_type, b_cast) in &types {
                let columns = vec![
                    TypedSource {
                        name: "a".to_string(),
                        data_type: a_type.clone(),
                        cast_sql: a_cast.to_string(),
                    },
                    TypedSource {
                        name: "b".to_string(),
                        data_type: b_type.clone(),
                        cast_sql: b_cast.to_string(),
                    },
                ];
                let expr_sql = format!("{func}(a, b)");
                let sql = format!(
                    "WITH data AS (SELECT {a_cast} AS a, {b_cast} AS b) SELECT {expr_sql} AS expr_0 FROM data"
                );
                run_case(
                    &sql,
                    &expr_sql,
                    &columns,
                    &format!("{func}({a_label}, {b_label})"),
                    &mut failures,
                );
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} numeric two-arg function type mismatch(es):\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}
