//! Type-aware SQL expression generators for property-based testing.
//!
//! Generates random typed CTE queries like:
//! ```sql
//! WITH data AS (
//!   SELECT CAST(42 AS INTEGER) AS int_col, CAST('hello' AS VARCHAR) AS str_col
//! )
//! SELECT LENGTH(str_col) AS expr_0, int_col + 1 AS expr_1 FROM data
//! ```
//!
//! Each generated expression carries its expected smelt type so the property test
//! can compare smelt's inference against DuckDB's actual type.

use proptest::prelude::*;
use smelt_types::{DataType, SqlFunction};

/// A typed column in the CTE source.
#[derive(Debug, Clone)]
pub struct TypedSource {
    pub name: String,
    pub data_type: DataType,
    pub cast_sql: String,
}

/// A generated expression with its expected smelt-inferred type.
#[derive(Debug, Clone)]
pub struct TypedExpr {
    pub sql: String,
    pub alias: String,
    pub expected_smelt_type: DataType,
}

// ---- Base type generators ----

/// The base types we can generate columns for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BaseType {
    Boolean,
    Integer,
    BigInt,
    Double,
    Varchar,
    Date,
    Timestamp,
    Decimal,
}

impl BaseType {
    pub fn all() -> &'static [BaseType] {
        &[
            BaseType::Boolean,
            BaseType::Integer,
            BaseType::BigInt,
            BaseType::Double,
            BaseType::Varchar,
            BaseType::Date,
            BaseType::Timestamp,
            BaseType::Decimal,
        ]
    }

    pub fn to_smelt_type(self) -> DataType {
        match self {
            BaseType::Boolean => DataType::Boolean,
            BaseType::Integer => DataType::Integer,
            BaseType::BigInt => DataType::BigInt,
            BaseType::Double => DataType::Double,
            BaseType::Varchar => DataType::Varchar { max_length: None },
            BaseType::Date => DataType::Date,
            BaseType::Timestamp => DataType::Timestamp {
                with_timezone: false,
            },
            BaseType::Decimal => DataType::Decimal {
                precision: 10,
                scale: 2,
            },
        }
    }

    pub fn cast_sql(self) -> &'static str {
        match self {
            BaseType::Boolean => "CAST(TRUE AS BOOLEAN)",
            BaseType::Integer => "CAST(42 AS INTEGER)",
            BaseType::BigInt => "CAST(100 AS BIGINT)",
            BaseType::Double => "CAST(3.14 AS DOUBLE)",
            BaseType::Varchar => "CAST('hello' AS STRING)",
            BaseType::Date => "CAST('2024-01-01' AS DATE)",
            BaseType::Timestamp => "CAST('2024-01-01 12:00:00' AS TIMESTAMP)",
            BaseType::Decimal => "CAST(99.99 AS DECIMAL(10,2))",
        }
    }

    pub fn col_prefix(self) -> &'static str {
        match self {
            BaseType::Boolean => "bool_col",
            BaseType::Integer => "int_col",
            BaseType::BigInt => "bigint_col",
            BaseType::Double => "dbl_col",
            BaseType::Varchar => "str_col",
            BaseType::Date => "date_col",
            BaseType::Timestamp => "ts_col",
            BaseType::Decimal => "dec_col",
        }
    }
}

/// An expression kind the generator can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExprKind {
    /// Direct column reference.
    ColumnRef,
    /// CAST(col AS type).
    Cast,
    /// Function call on compatible columns.
    Function,
    /// Binary arithmetic/string operation.
    BinaryOp,
    /// CASE WHEN ... THEN ... ELSE ... END.
    CaseExpr,
}

// ---- Function descriptors ----

/// A function we can generate, with its required input type and output type.
#[derive(Debug, Clone)]
pub struct FuncDesc {
    pub name: &'static str,
    pub input: FuncInput,
    pub output_type: DataType,
}

#[derive(Debug, Clone, Copy)]
pub enum FuncInput {
    /// Function takes a string argument.
    String,
    /// Function takes a numeric argument.
    Numeric,
    /// Function takes a temporal argument (date/timestamp).
    Temporal,
    /// Aggregate that takes any type.
    AnyAggregate,
    /// Aggregate on numeric.
    NumericAggregate,
    /// Non-aggregate function that takes any type.
    AnyScalar,
}

/// Core functions we test. Expand this list over time.
pub fn core_functions() -> Vec<FuncDesc> {
    vec![
        // String functions -> return varchar in DuckDB
        FuncDesc {
            name: "UPPER",
            input: FuncInput::String,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "LOWER",
            input: FuncInput::String,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "TRIM",
            input: FuncInput::String,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "REVERSE",
            input: FuncInput::String,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "LTRIM",
            input: FuncInput::String,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "RTRIM",
            input: FuncInput::String,
            output_type: DataType::Text,
        },
        // INITCAP omitted: not available in DuckDB
        FuncDesc {
            name: "CONCAT",
            input: FuncInput::String,
            output_type: DataType::Text,
        },
        // String functions -> return integer in smelt
        FuncDesc {
            name: "LENGTH",
            input: FuncInput::String,
            output_type: DataType::BigInt,
        },
        FuncDesc {
            name: "CHAR_LENGTH",
            input: FuncInput::String,
            output_type: DataType::BigInt,
        },
        FuncDesc {
            name: "CHARACTER_LENGTH",
            input: FuncInput::String,
            output_type: DataType::BigInt,
        },
        // Numeric functions
        FuncDesc {
            name: "ABS",
            input: FuncInput::Numeric,
            output_type: DataType::Double, // fallback; actual is arg-dependent
        },
        FuncDesc {
            name: "CEIL",
            input: FuncInput::Numeric,
            output_type: DataType::Double, // fallback
        },
        FuncDesc {
            name: "FLOOR",
            input: FuncInput::Numeric,
            output_type: DataType::Double, // fallback
        },
        FuncDesc {
            name: "SQRT",
            input: FuncInput::Numeric,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "ROUND",
            input: FuncInput::Numeric,
            output_type: DataType::Double, // fallback
        },
        FuncDesc {
            name: "SIGN",
            input: FuncInput::Numeric,
            output_type: DataType::SmallInt,
        },
        // POWER/POW omitted: requires 2 args (added in multi-arg step)
        // Math functions -> always return Double
        FuncDesc {
            name: "EXP",
            input: FuncInput::Numeric,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "LN",
            input: FuncInput::Numeric,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "LOG",
            input: FuncInput::Numeric,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "LOG10",
            input: FuncInput::Numeric,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "LOG2",
            input: FuncInput::Numeric,
            output_type: DataType::Double,
        },
        // Trigonometric functions -> always return Double
        FuncDesc {
            name: "SIN",
            input: FuncInput::Numeric,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "COS",
            input: FuncInput::Numeric,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "TAN",
            input: FuncInput::Numeric,
            output_type: DataType::Double,
        },
        // ASIN and ACOS omitted: require input in [-1,1], sample values cause domain errors
        FuncDesc {
            name: "ATAN",
            input: FuncInput::Numeric,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "SINH",
            input: FuncInput::Numeric,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "COSH",
            input: FuncInput::Numeric,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "TANH",
            input: FuncInput::Numeric,
            output_type: DataType::Double,
        },
        // Aggregates
        FuncDesc {
            name: "COUNT",
            input: FuncInput::AnyAggregate,
            output_type: DataType::BigInt,
        },
        FuncDesc {
            name: "SUM",
            input: FuncInput::NumericAggregate,
            output_type: DataType::Decimal {
                precision: 38,
                scale: 10,
            },
        },
        FuncDesc {
            name: "AVG",
            input: FuncInput::NumericAggregate,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "MIN",
            input: FuncInput::AnyAggregate,
            output_type: DataType::Unknown, // arg-dependent
        },
        FuncDesc {
            name: "MAX",
            input: FuncInput::AnyAggregate,
            output_type: DataType::Unknown, // arg-dependent
        },
        // Null-handling / scalar functions that accept any type
        FuncDesc {
            name: "COALESCE",
            input: FuncInput::AnyScalar,
            output_type: DataType::Unknown, // arg-dependent
        },
        // Comparison functions
        FuncDesc {
            name: "GREATEST",
            input: FuncInput::AnyScalar,
            output_type: DataType::Unknown, // arg-dependent
        },
        FuncDesc {
            name: "LEAST",
            input: FuncInput::AnyScalar,
            output_type: DataType::Unknown, // arg-dependent
        },
        // Statistical aggregates -> return Double
        FuncDesc {
            name: "STDDEV",
            input: FuncInput::NumericAggregate,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "VARIANCE",
            input: FuncInput::NumericAggregate,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "STDDEV_POP",
            input: FuncInput::NumericAggregate,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "STDDEV_SAMP",
            input: FuncInput::NumericAggregate,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "VAR_POP",
            input: FuncInput::NumericAggregate,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "VAR_SAMP",
            input: FuncInput::NumericAggregate,
            output_type: DataType::Double,
        },
    ]
}

/// Check if a BaseType is compatible with a FuncInput.
pub fn is_compatible(base: BaseType, input: FuncInput) -> bool {
    match input {
        FuncInput::String => base == BaseType::Varchar,
        FuncInput::Numeric => matches!(
            base,
            BaseType::Integer | BaseType::BigInt | BaseType::Double | BaseType::Decimal
        ),
        FuncInput::Temporal => matches!(base, BaseType::Date | BaseType::Timestamp),
        FuncInput::AnyScalar | FuncInput::AnyAggregate => true,
        FuncInput::NumericAggregate => matches!(
            base,
            BaseType::Integer | BaseType::BigInt | BaseType::Double | BaseType::Decimal
        ),
    }
}

/// Determine the smelt-inferred return type for a function given its input type.
/// For functions whose return type depends on the argument (like ABS, MIN, MAX),
/// use the argument type.
pub fn function_return_type(func_name: &str, arg_type: &DataType) -> DataType {
    match func_name {
        "ABS" | "ROUND" | "TRUNC" => arg_type.clone(),
        "SIGN" => DataType::SmallInt,
        "CEIL" | "CEILING" | "FLOOR" => match arg_type {
            DataType::Decimal { precision, .. } => DataType::Decimal {
                precision: *precision,
                scale: 0,
            },
            _ => DataType::Double,
        },
        "MIN" | "MAX" | "COALESCE" | "NULLIF" | "GREATEST" | "LEAST" => arg_type.clone(),
        "COUNT" => DataType::BigInt,
        "SUM" => DataType::Decimal {
            precision: 38,
            scale: 10,
        },
        "AVG" | "STDDEV" | "VARIANCE" | "STDDEV_POP" | "STDDEV_SAMP" | "VAR_POP" | "VAR_SAMP" => {
            DataType::Double
        }
        "LENGTH" | "CHAR_LENGTH" | "CHARACTER_LENGTH" => DataType::BigInt,
        "SQRT" | "EXP" | "LN" | "LOG" | "LOG10" | "LOG2" | "POWER" | "POW" | "SIN" | "COS"
        | "TAN" | "ASIN" | "ACOS" | "ATAN" | "SINH" | "COSH" | "TANH" => DataType::Double,
        // String functions
        "UPPER" | "LOWER" | "TRIM" | "LTRIM" | "RTRIM" | "REVERSE" | "CONCAT" | "REPLACE"
        | "REPEAT" | "LPAD" | "RPAD" | "INITCAP" | "SUBSTRING" | "SUBSTR" | "LEFT" | "RIGHT"
        | "SPLIT_PART" => DataType::Text,
        _ => DataType::Unknown,
    }
}

// ---- Proptest strategies ----

/// Strategy that picks a random BaseType.
pub fn base_type_strategy() -> impl Strategy<Value = BaseType> {
    prop::sample::select(BaseType::all())
}

/// Strategy that generates a pool of 1-5 typed columns.
pub fn column_pool_strategy() -> impl Strategy<Value = Vec<TypedSource>> {
    prop::collection::vec(base_type_strategy(), 1..=5).prop_map(|types| {
        types
            .into_iter()
            .enumerate()
            .map(|(i, bt)| TypedSource {
                name: format!("{}_{}", bt.col_prefix(), i),
                data_type: bt.to_smelt_type(),
                cast_sql: bt.cast_sql().to_string(),
            })
            .collect()
    })
}

/// Strategy that picks an expression kind.
pub fn expr_kind_strategy() -> impl Strategy<Value = ExprKind> {
    prop_oneof![
        2 => Just(ExprKind::ColumnRef),
        3 => Just(ExprKind::Function),
        2 => Just(ExprKind::BinaryOp),
        1 => Just(ExprKind::CaseExpr),
        1 => Just(ExprKind::Cast),
    ]
}

/// Generate a typed expression given a column pool and expression kind.
/// Returns None if the expression kind is not compatible with available columns.
pub fn generate_expr(
    columns: &[TypedSource],
    kind: ExprKind,
    expr_idx: usize,
    func_idx: usize,
) -> Option<TypedExpr> {
    let alias = format!("expr_{}", expr_idx);

    match kind {
        ExprKind::ColumnRef => {
            let col = &columns[expr_idx % columns.len()];
            Some(TypedExpr {
                sql: col.name.clone(),
                alias,
                expected_smelt_type: col.data_type.clone(),
            })
        }

        ExprKind::Cast => {
            // Cast first column to a different type
            let col = &columns[expr_idx % columns.len()];
            // Cast numerics to DOUBLE, strings to VARCHAR, others to VARCHAR
            let (cast_type, smelt_type) = if col.data_type.is_numeric() {
                ("DOUBLE", DataType::Double)
            } else {
                ("STRING", DataType::Varchar { max_length: None })
            };
            Some(TypedExpr {
                sql: format!("CAST({} AS {})", col.name, cast_type),
                alias,
                expected_smelt_type: smelt_type,
            })
        }

        ExprKind::Function => {
            let funcs = core_functions();
            let func = &funcs[func_idx % funcs.len()];

            // Find a compatible column
            let compatible_col = columns.iter().find(|c| {
                smelt_type_to_base(&c.data_type).is_some_and(|b| is_compatible(b, func.input))
            })?;

            let return_type = function_return_type(func.name, &compatible_col.data_type);

            let sql = format!("{}({})", func.name, compatible_col.name);

            Some(TypedExpr {
                sql,
                alias,
                expected_smelt_type: return_type,
            })
        }

        ExprKind::BinaryOp => {
            // Find a numeric column for arithmetic, or string for ||
            if let Some(num_col) = columns.iter().find(|c| c.data_type.is_numeric()) {
                // integer + integer => same type promotion
                let expected = num_col.data_type.clone();
                Some(TypedExpr {
                    sql: format!("{} + {}", num_col.name, num_col.name),
                    alias,
                    expected_smelt_type: expected,
                })
            } else {
                columns
                    .iter()
                    .find(|c| c.data_type.is_string())
                    .map(|str_col| TypedExpr {
                        sql: format!("{} || {}", str_col.name, str_col.name),
                        alias,
                        expected_smelt_type: DataType::Text,
                    })
            }
        }

        ExprKind::CaseExpr => {
            let col = &columns[expr_idx % columns.len()];
            Some(TypedExpr {
                sql: format!("CASE WHEN TRUE THEN {} ELSE {} END", col.name, col.name),
                alias,
                expected_smelt_type: col.data_type.clone(),
            })
        }
    }
}

fn smelt_type_to_base(dt: &DataType) -> Option<BaseType> {
    match dt {
        DataType::Boolean => Some(BaseType::Boolean),
        DataType::Integer => Some(BaseType::Integer),
        DataType::BigInt => Some(BaseType::BigInt),
        DataType::Double | DataType::Float => Some(BaseType::Double),
        DataType::Varchar { .. } | DataType::Text | DataType::Char { .. } => {
            Some(BaseType::Varchar)
        }
        DataType::Date => Some(BaseType::Date),
        DataType::Timestamp { .. } => Some(BaseType::Timestamp),
        DataType::Decimal { .. } => Some(BaseType::Decimal),
        DataType::SmallInt => Some(BaseType::Integer),
        _ => None,
    }
}

/// Check if a SQL expression string is an aggregate function call.
fn is_aggregate_expr(sql: &str) -> bool {
    let upper = sql.to_uppercase();
    if let Some(paren_pos) = upper.find('(') {
        let name = upper[..paren_pos].trim();
        SqlFunction::from_name(name).is_some_and(|f| f.is_aggregate())
    } else {
        false
    }
}

/// Assemble a CTE query from columns and expressions.
///
/// If any expression uses aggregate functions, we wrap the whole SELECT in a
/// GROUP BY on all non-aggregate columns (or just use SELECT without FROM for
/// a single-row aggregate).
pub fn assemble_cte_query(columns: &[TypedSource], exprs: &[TypedExpr]) -> String {
    // Build the CTE
    let cte_cols: Vec<String> = columns
        .iter()
        .map(|c| format!("{} AS {}", c.cast_sql, c.name))
        .collect();

    let select_exprs: Vec<String> = exprs
        .iter()
        .map(|e| format!("{} AS {}", e.sql, e.alias))
        .collect();

    let has_aggregate = exprs.iter().any(|e| is_aggregate_expr(&e.sql));

    if has_aggregate {
        // For queries with aggregates, only include aggregate expressions
        let agg_exprs: Vec<String> = exprs
            .iter()
            .filter(|e| is_aggregate_expr(&e.sql))
            .map(|e| format!("{} AS {}", e.sql, e.alias))
            .collect();

        if agg_exprs.is_empty() {
            format!(
                "WITH data AS (SELECT {}) SELECT {} FROM data",
                cte_cols.join(", "),
                select_exprs.join(", ")
            )
        } else {
            format!(
                "WITH data AS (SELECT {}) SELECT {} FROM data",
                cte_cols.join(", "),
                agg_exprs.join(", ")
            )
        }
    } else {
        format!(
            "WITH data AS (SELECT {}) SELECT {} FROM data",
            cte_cols.join(", "),
            select_exprs.join(", ")
        )
    }
}

/// Strategy for generating complete test scenarios.
pub fn test_scenario_strategy(
) -> impl Strategy<Value = (Vec<TypedSource>, Vec<ExprKind>, Vec<usize>)> {
    column_pool_strategy().prop_flat_map(|cols| {
        let num_exprs = 1..=4usize;
        (
            Just(cols),
            prop::collection::vec(expr_kind_strategy(), num_exprs.clone()),
            prop::collection::vec(0..100usize, 1..=4),
        )
            .prop_filter("need at least one expr", |(_, kinds, _)| !kinds.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assemble_simple_query() {
        let cols = vec![TypedSource {
            name: "int_col_0".into(),
            data_type: DataType::Integer,
            cast_sql: "CAST(42 AS INTEGER)".into(),
        }];
        let exprs = vec![TypedExpr {
            sql: "int_col_0".into(),
            alias: "expr_0".into(),
            expected_smelt_type: DataType::Integer,
        }];
        let sql = assemble_cte_query(&cols, &exprs);
        assert!(sql.contains("WITH data AS"));
        assert!(sql.contains("CAST(42 AS INTEGER) AS int_col_0"));
        assert!(sql.contains("int_col_0 AS expr_0"));
    }

    #[test]
    fn assemble_aggregate_query() {
        let cols = vec![TypedSource {
            name: "int_col_0".into(),
            data_type: DataType::Integer,
            cast_sql: "CAST(42 AS INTEGER)".into(),
        }];
        let exprs = vec![TypedExpr {
            sql: "COUNT(int_col_0)".into(),
            alias: "expr_0".into(),
            expected_smelt_type: DataType::BigInt,
        }];
        let sql = assemble_cte_query(&cols, &exprs);
        assert!(sql.contains("COUNT(int_col_0) AS expr_0"));
    }

    #[test]
    fn generate_column_ref() {
        let cols = vec![TypedSource {
            name: "x".into(),
            data_type: DataType::Integer,
            cast_sql: "CAST(1 AS INTEGER)".into(),
        }];
        let expr = generate_expr(&cols, ExprKind::ColumnRef, 0, 0).unwrap();
        assert_eq!(expr.sql, "x");
        assert_eq!(expr.expected_smelt_type, DataType::Integer);
    }
}
