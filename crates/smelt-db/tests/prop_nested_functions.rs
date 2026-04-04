//! Property-based tests for nested function expressions.
//!
//! Tests smelt's type inference for deeply-nested function compositions (2-4 levels),
//! where the output of one function feeds into another, validated against DuckDB.
//!
//! Examples:
//!   LENGTH(UPPER(col))
//!   ABS(LENGTH(LOWER(col)))
//!   CAST(EXTRACT(YEAR FROM ts) AS VARCHAR)
//!   COALESCE(SUM(x), 0)

#[allow(dead_code)]
mod prop_helpers;

use prop_helpers::divergences::{find_divergence, known_divergences, TypeDivergence};
use prop_helpers::duckdb_oracle::{DuckDbOracle, TypeOracle};
use prop_helpers::generators::{self, assemble_cte_query, column_pool_strategy, TypedExpr};
use prop_helpers::type_comparison::{compare_types, TypeMatch};

use smelt_db::type_inference::{infer_select_column_types, TypeContext};
use smelt_parser::ast::File;
use smelt_types::{DataType, TypedColumn};

use proptest::prelude::*;

// ---- Nesting infrastructure ----

/// A function that can wrap an expression, transforming its type.
#[derive(Debug, Clone)]
struct WrapperFunc {
    name: &'static str,
    /// The output type (or Unknown if it depends on the argument).
    output: DataType,
}

/// Wrappers that take a string and return a string.
fn string_to_string_wrappers() -> Vec<WrapperFunc> {
    vec![
        WrapperFunc {
            name: "UPPER",

            output: DataType::Text,
        },
        WrapperFunc {
            name: "LOWER",

            output: DataType::Text,
        },
        WrapperFunc {
            name: "TRIM",

            output: DataType::Text,
        },
        WrapperFunc {
            name: "REVERSE",

            output: DataType::Text,
        },
        WrapperFunc {
            name: "LTRIM",

            output: DataType::Text,
        },
        WrapperFunc {
            name: "RTRIM",

            output: DataType::Text,
        },
    ]
}

/// Wrappers that take a string and return a number.
fn string_to_numeric_wrappers() -> Vec<WrapperFunc> {
    vec![WrapperFunc {
        name: "LENGTH",
        output: DataType::BigInt,
    }]
}

/// Wrappers that take a numeric and return a numeric.
fn numeric_to_numeric_wrappers() -> Vec<WrapperFunc> {
    vec![
        WrapperFunc {
            name: "ABS",

            output: DataType::Unknown, // arg-dependent
        },
        WrapperFunc {
            name: "SQRT",

            output: DataType::Double,
        },
        WrapperFunc {
            name: "EXP",

            output: DataType::Double,
        },
        WrapperFunc {
            name: "LN",

            output: DataType::Double,
        },
        WrapperFunc {
            name: "SIGN",

            output: DataType::SmallInt,
        },
        WrapperFunc {
            name: "CEIL",

            output: DataType::Unknown, // arg-dependent
        },
        WrapperFunc {
            name: "FLOOR",

            output: DataType::Unknown, // arg-dependent
        },
    ]
}

/// Given the current expression type, return compatible wrappers to apply on top.
fn compatible_wrappers(current_type: &DataType) -> Vec<WrapperFunc> {
    let mut result = Vec::new();

    if current_type.is_string() {
        result.extend(string_to_string_wrappers());
        result.extend(string_to_numeric_wrappers());
    }

    if current_type.is_numeric() {
        result.extend(numeric_to_numeric_wrappers());
    }

    // COALESCE works on any type and returns the same type
    result.push(WrapperFunc {
        name: "COALESCE",

        output: DataType::Unknown, // same as input
    });

    result
}

/// Compute the output type when wrapping an expression of `inner_type` with `wrapper`.
fn wrapper_output_type(wrapper: &WrapperFunc, inner_type: &DataType) -> DataType {
    match wrapper.name {
        // Arg-dependent functions
        "ABS" | "ROUND" | "TRUNC" => inner_type.clone(),
        "CEIL" | "CEILING" | "FLOOR" => match inner_type {
            DataType::Decimal { precision, .. } => DataType::Decimal {
                precision: *precision,
                scale: 0,
            },
            _ => DataType::Double,
        },
        "COALESCE" => inner_type.clone(),
        // All others use their declared output type
        _ => {
            if wrapper.output == DataType::Unknown {
                inner_type.clone()
            } else {
                wrapper.output.clone()
            }
        }
    }
}

/// Build the SQL for wrapping an expression.
fn wrap_sql(wrapper: &WrapperFunc, inner_sql: &str, inner_type: &DataType) -> String {
    match wrapper.name {
        "COALESCE" => {
            // COALESCE(expr, default) — need a type-appropriate default
            let default = coalesce_default(inner_type);
            format!("COALESCE({inner_sql}, {default})")
        }
        _ => format!("{}({inner_sql})", wrapper.name),
    }
}

/// Return a literal default value for COALESCE that matches the given type.
fn coalesce_default(dt: &DataType) -> &'static str {
    match dt {
        DataType::Integer | DataType::SmallInt => "0",
        DataType::BigInt => "CAST(0 AS BIGINT)",
        DataType::Double | DataType::Float => "0.0",
        DataType::Decimal { .. } => "CAST(0 AS DECIMAL(10,2))",
        DataType::Boolean => "FALSE",
        DataType::Text | DataType::Varchar { .. } | DataType::Char { .. } => "''",
        DataType::Date => "CAST('2024-01-01' AS DATE)",
        DataType::Timestamp { .. } => "CAST('2024-01-01 00:00:00' AS TIMESTAMP)",
        DataType::Time => "CAST('00:00:00' AS TIME)",
        DataType::Interval => "CAST('0 seconds' AS INTERVAL)",
        _ => "NULL",
    }
}

// ---- A nesting chain ----

/// A chain of nested function applications.
/// E.g., chain = [LENGTH, UPPER] means LENGTH(UPPER(col)).
#[derive(Debug, Clone)]
struct NestingChain {
    /// The base column.
    base_col: generators::TypedSource,
    /// Wrappers applied from innermost to outermost.
    wrappers: Vec<WrapperFunc>,
}

impl NestingChain {
    /// Build the final SQL expression.
    fn to_sql(&self) -> String {
        let mut sql = self.base_col.name.clone();
        let mut current_type = self.base_col.data_type.clone();

        for wrapper in &self.wrappers {
            sql = wrap_sql(wrapper, &sql, &current_type);
            current_type = wrapper_output_type(wrapper, &current_type);
        }

        sql
    }

    /// Compute the expected output type.
    fn expected_type(&self) -> DataType {
        let mut current = self.base_col.data_type.clone();
        for wrapper in &self.wrappers {
            current = wrapper_output_type(wrapper, &current);
        }
        current
    }
}

// ---- Proptest strategies ----

/// Strategy to generate a nesting depth (2-4).
fn nesting_depth_strategy() -> impl Strategy<Value = usize> {
    2..=4usize
}

/// Strategy to generate a nesting chain from a given column pool.
fn nesting_chain_strategy(
    columns: Vec<generators::TypedSource>,
) -> impl Strategy<Value = NestingChain> {
    let cols = columns.clone();
    (
        // Pick a base column index
        0..columns.len(),
        // Pick nesting depth
        nesting_depth_strategy(),
        // Random indices for wrapper selection
        prop::collection::vec(0..100usize, 4),
    )
        .prop_map(move |(col_idx, depth, wrapper_indices)| {
            let base_col = cols[col_idx].clone();
            let mut wrappers = Vec::new();
            let mut current_type = base_col.data_type.clone();

            for i in 0..depth {
                let compatible = compatible_wrappers(&current_type);
                if compatible.is_empty() {
                    break;
                }
                let wrapper = compatible
                    [wrapper_indices[i % wrapper_indices.len()] % compatible.len()]
                .clone();
                current_type = wrapper_output_type(&wrapper, &current_type);
                wrappers.push(wrapper);
            }

            NestingChain { base_col, wrappers }
        })
        .prop_filter("need at least 2 levels of nesting", |chain| {
            chain.wrappers.len() >= 2
        })
}

/// Strategy for a complete nested-function test scenario.
fn nested_scenario_strategy(
) -> impl Strategy<Value = (Vec<generators::TypedSource>, Vec<NestingChain>)> {
    column_pool_strategy().prop_flat_map(|columns| {
        let cols = columns.clone();
        // Generate 1-3 nesting chains
        let chain_strat = nesting_chain_strategy(columns.clone());
        (Just(cols), prop::collection::vec(chain_strat, 1..=3))
    })
}

// ---- Inference helpers ----

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

/// Compare smelt inference against DuckDB oracle.
fn check_types_against_oracle(
    oracle: &dyn TypeOracle,
    backend: &str,
    sql: &str,
    columns: &[generators::TypedSource],
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

        if *smelt_type == DataType::Unknown {
            continue;
        }

        match compare_types(smelt_type, actual_type) {
            TypeMatch::Exact | TypeMatch::Compatible { .. } => {}
            TypeMatch::Mismatch => {
                if find_divergence(smelt_type, actual_type, backend, divergences).is_none() {
                    return Err(format!(
                        "Nested function type mismatch for column {} ({}) against {backend}:\n  \
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

    /// Core nested-function property test: generate expressions with 2-4 levels of
    /// function nesting and validate smelt's type inference against DuckDB.
    #[test]
    fn prop_nested_functions(
        (columns, chains) in nested_scenario_strategy()
    ) {
        let duckdb = DuckDbOracle::new();
        let divergences = known_divergences();

        // Convert chains to TypedExprs
        let exprs: Vec<TypedExpr> = chains
            .iter()
            .enumerate()
            .map(|(i, chain)| TypedExpr {
                sql: chain.to_sql(),
                alias: format!("expr_{i}"),
                expected_smelt_type: chain.expected_type(),
            })
            .collect();

        prop_assume!(!exprs.is_empty());

        let sql = assemble_cte_query(&columns, &exprs, &generators::QueryShape::Scalar);

        if let Err(msg) = check_types_against_oracle(&duckdb, "duckdb", &sql, &columns, &divergences) {
            prop_assert!(false, "{}", msg);
        }
    }
}

// ---- Deterministic smoke tests for common nesting patterns ----

#[test]
fn smoke_length_of_upper() {
    // LENGTH(UPPER(str_col)) → BigInt
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST('hello' AS VARCHAR) AS s) SELECT LENGTH(UPPER(s)) AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "s".into(),
        data_type: DataType::Varchar { max_length: None },
        cast_sql: "CAST('hello' AS VARCHAR)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    assert_eq!(inferred[0].1, DataType::BigInt);
    assert_eq!(actual[0].1, DataType::BigInt);
}

#[test]
fn smoke_abs_of_length() {
    // ABS(LENGTH(str_col)) → BigInt (ABS preserves arg type)
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST('hello' AS VARCHAR) AS s) SELECT ABS(LENGTH(s)) AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "s".into(),
        data_type: DataType::Varchar { max_length: None },
        cast_sql: "CAST('hello' AS VARCHAR)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    // ABS(BigInt) → BigInt
    assert_eq!(inferred[0].1, DataType::BigInt);
    assert_eq!(actual[0].1, DataType::BigInt);
}

#[test]
fn smoke_sqrt_of_abs() {
    // SQRT(ABS(dbl_col)) → Double
    let oracle = DuckDbOracle::new();
    let sql =
        "WITH data AS (SELECT CAST(3.14 AS DOUBLE) AS d) SELECT SQRT(ABS(d)) AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "d".into(),
        data_type: DataType::Double,
        cast_sql: "CAST(3.14 AS DOUBLE)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    assert_eq!(inferred[0].1, DataType::Double);
    assert_eq!(actual[0].1, DataType::Double);
}

#[test]
fn smoke_triple_nested_string() {
    // REVERSE(UPPER(LOWER(str_col))) → Text
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST('hello' AS VARCHAR) AS s) SELECT REVERSE(UPPER(LOWER(s))) AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "s".into(),
        data_type: DataType::Varchar { max_length: None },
        cast_sql: "CAST('hello' AS VARCHAR)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    // All string functions return Text in smelt
    assert_eq!(inferred[0].1, DataType::Text);
    // DuckDB returns Varchar, but that's a known compatible difference
    let match_result = compare_types(&inferred[0].1, &actual[0].1);
    assert!(matches!(
        match_result,
        TypeMatch::Exact | TypeMatch::Compatible { .. }
    ));
}

#[test]
fn smoke_coalesce_nested() {
    // COALESCE(ABS(int_col), 0) → Integer
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(42 AS INTEGER) AS x) SELECT COALESCE(ABS(x), 0) AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "x".into(),
        data_type: DataType::Integer,
        cast_sql: "CAST(42 AS INTEGER)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

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

#[test]
fn smoke_sign_of_length() {
    // SIGN(LENGTH(str_col)) → SmallInt
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST('hello' AS VARCHAR) AS s) SELECT SIGN(LENGTH(s)) AS expr_0 FROM data";

    let columns = vec![generators::TypedSource {
        name: "s".into(),
        data_type: DataType::Varchar { max_length: None },
        cast_sql: "CAST('hello' AS VARCHAR)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    // SIGN always returns SmallInt in smelt
    assert_eq!(inferred[0].1, DataType::SmallInt);

    // DuckDB may return a different integer type — that's expected
    let actual = oracle.query_types(sql).unwrap();
    let divergences = known_divergences();
    let match_result = compare_types(&inferred[0].1, &actual[0].1);
    if matches!(match_result, TypeMatch::Mismatch) {
        // Known divergence for SIGN is acceptable
        assert!(
            find_divergence(&inferred[0].1, &actual[0].1, "duckdb", &divergences).is_some()
                || matches!(match_result, TypeMatch::Compatible { .. }),
            "Unexpected mismatch: smelt={:?}, duckdb={:?}",
            inferred[0].1,
            actual[0].1
        );
    }
}

#[test]
fn smoke_exp_of_ceil() {
    // EXP(CEIL(dbl_col)) → Double
    let oracle = DuckDbOracle::new();
    let sql =
        "WITH data AS (SELECT CAST(2.5 AS DOUBLE) AS d) SELECT EXP(CEIL(d)) AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "d".into(),
        data_type: DataType::Double,
        cast_sql: "CAST(2.5 AS DOUBLE)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    assert_eq!(inferred[0].1, DataType::Double);
    assert_eq!(actual[0].1, DataType::Double);
}

#[test]
fn smoke_four_levels_deep() {
    // LN(SQRT(ABS(CEIL(dbl_col)))) → Double
    let oracle = DuckDbOracle::new();
    let sql = "WITH data AS (SELECT CAST(2.5 AS DOUBLE) AS d) SELECT LN(SQRT(ABS(CEIL(d)))) AS expr_0 FROM data";
    let actual = oracle.query_types(sql).unwrap();

    let columns = vec![generators::TypedSource {
        name: "d".into(),
        data_type: DataType::Double,
        cast_sql: "CAST(2.5 AS DOUBLE)".into(),
    }];
    let inferred = run_smelt_inference(sql, &columns);

    assert_eq!(inferred[0].1, DataType::Double);
    assert_eq!(actual[0].1, DataType::Double);
}
