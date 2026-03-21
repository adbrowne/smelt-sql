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
    /// col BETWEEN val AND val.
    Between,
    /// col IN (val1, val2, ...).
    InList,
    /// JSON operator (-> or ->>).
    JsonOp,
}

// ---- Function descriptors ----

/// Additional arguments beyond the first column argument.
#[derive(Debug, Clone, Copy)]
pub enum ExtraArg {
    /// Re-use the same column as the first argument.
    SameAsFirst,
    /// An integer literal.
    IntLiteral(&'static str),
    /// A string literal (will be single-quoted).
    StringLiteral(&'static str),
}

/// A function we can generate, with its required input type and output type.
#[derive(Debug, Clone)]
pub struct FuncDesc {
    pub name: &'static str,
    pub input: FuncInput,
    pub extra_args: &'static [ExtraArg],
    /// If set, this string literal is prepended before the column argument.
    pub prepend_literal: Option<&'static str>,
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
    /// Aggregate on boolean columns.
    BooleanAggregate,
    /// Aggregate on integer columns (Integer/BigInt).
    IntegerAggregate,
    /// Function takes no arguments.
    NoArg,
}

/// Core functions we test. Expand this list over time.
pub fn core_functions() -> Vec<FuncDesc> {
    vec![
        // String functions -> return varchar in DuckDB
        FuncDesc {
            name: "UPPER",
            input: FuncInput::String,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "LOWER",
            input: FuncInput::String,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "TRIM",
            input: FuncInput::String,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "REVERSE",
            input: FuncInput::String,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "LTRIM",
            input: FuncInput::String,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "RTRIM",
            input: FuncInput::String,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        // INITCAP omitted: not available in DuckDB
        FuncDesc {
            name: "CONCAT",
            input: FuncInput::String,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        // String functions -> return integer in smelt
        FuncDesc {
            name: "LENGTH",
            input: FuncInput::String,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::BigInt,
        },
        FuncDesc {
            name: "CHAR_LENGTH",
            input: FuncInput::String,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::BigInt,
        },
        FuncDesc {
            name: "CHARACTER_LENGTH",
            input: FuncInput::String,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::BigInt,
        },
        // Numeric functions
        FuncDesc {
            name: "ABS",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double, // fallback; actual is arg-dependent
        },
        FuncDesc {
            name: "CEIL",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double, // fallback
        },
        FuncDesc {
            name: "FLOOR",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double, // fallback
        },
        FuncDesc {
            name: "SQRT",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "ROUND",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double, // fallback
        },
        FuncDesc {
            name: "SIGN",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::SmallInt,
        },
        // POWER/POW omitted: requires 2 args (added in multi-arg step)
        // Math functions -> always return Double
        FuncDesc {
            name: "EXP",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "LN",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "LOG",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "LOG10",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "LOG2",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        // Trigonometric functions -> always return Double
        FuncDesc {
            name: "SIN",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "COS",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "TAN",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        // ASIN and ACOS omitted: require input in [-1,1], sample values cause domain errors
        FuncDesc {
            name: "ATAN",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "SINH",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "COSH",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "TANH",
            input: FuncInput::Numeric,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        // Aggregates
        FuncDesc {
            name: "COUNT",
            input: FuncInput::AnyAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::BigInt,
        },
        FuncDesc {
            name: "SUM",
            input: FuncInput::NumericAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Decimal {
                precision: 38,
                scale: 10,
            },
        },
        FuncDesc {
            name: "AVG",
            input: FuncInput::NumericAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "MIN",
            input: FuncInput::AnyAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Unknown, // arg-dependent
        },
        FuncDesc {
            name: "MAX",
            input: FuncInput::AnyAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Unknown, // arg-dependent
        },
        // Null-handling / scalar functions that accept any type
        FuncDesc {
            name: "COALESCE",
            input: FuncInput::AnyScalar,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Unknown, // arg-dependent
        },
        // Comparison functions
        FuncDesc {
            name: "GREATEST",
            input: FuncInput::AnyScalar,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Unknown, // arg-dependent
        },
        FuncDesc {
            name: "LEAST",
            input: FuncInput::AnyScalar,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Unknown, // arg-dependent
        },
        // Statistical aggregates -> return Double
        FuncDesc {
            name: "STDDEV",
            input: FuncInput::NumericAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "VARIANCE",
            input: FuncInput::NumericAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "STDDEV_POP",
            input: FuncInput::NumericAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "STDDEV_SAMP",
            input: FuncInput::NumericAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "VAR_POP",
            input: FuncInput::NumericAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "VAR_SAMP",
            input: FuncInput::NumericAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        // Boolean aggregates
        FuncDesc {
            name: "BOOL_AND",
            input: FuncInput::BooleanAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Boolean,
        },
        FuncDesc {
            name: "BOOL_OR",
            input: FuncInput::BooleanAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Boolean,
        },
        FuncDesc {
            name: "EVERY",
            input: FuncInput::BooleanAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Boolean,
        },
        // Zero-arg functions
        FuncDesc {
            name: "PI",
            input: FuncInput::NoArg,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        // Bit aggregates
        FuncDesc {
            name: "BIT_AND",
            input: FuncInput::IntegerAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Unknown, // arg-dependent
        },
        FuncDesc {
            name: "BIT_OR",
            input: FuncInput::IntegerAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Unknown, // arg-dependent
        },
        FuncDesc {
            name: "BIT_XOR",
            input: FuncInput::IntegerAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Unknown, // arg-dependent
        },
        // Multi-arg functions
        FuncDesc {
            name: "REPLACE",
            input: FuncInput::String,
            extra_args: &[ExtraArg::StringLiteral("l"), ExtraArg::StringLiteral("r")],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "LPAD",
            input: FuncInput::String,
            extra_args: &[ExtraArg::IntLiteral("10"), ExtraArg::StringLiteral("x")],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "RPAD",
            input: FuncInput::String,
            extra_args: &[ExtraArg::IntLiteral("10"), ExtraArg::StringLiteral("x")],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "LEFT",
            input: FuncInput::String,
            extra_args: &[ExtraArg::IntLiteral("3")],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "RIGHT",
            input: FuncInput::String,
            extra_args: &[ExtraArg::IntLiteral("3")],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "REPEAT",
            input: FuncInput::String,
            extra_args: &[ExtraArg::IntLiteral("2")],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "NULLIF",
            input: FuncInput::Numeric,
            extra_args: &[ExtraArg::IntLiteral("0")],
            prepend_literal: None,
            output_type: DataType::Unknown, // arg-dependent
        },
        FuncDesc {
            name: "POWER",
            input: FuncInput::Numeric,
            extra_args: &[ExtraArg::IntLiteral("2")],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "MOD",
            input: FuncInput::Numeric,
            extra_args: &[ExtraArg::SameAsFirst],
            prepend_literal: None,
            output_type: DataType::Unknown, // arg-dependent
        },
        FuncDesc {
            name: "ATAN2",
            input: FuncInput::Numeric,
            extra_args: &[ExtraArg::SameAsFirst],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        // Multi-arg string functions
        FuncDesc {
            name: "SUBSTRING",
            input: FuncInput::String,
            extra_args: &[ExtraArg::IntLiteral("1"), ExtraArg::IntLiteral("3")],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "SUBSTR",
            input: FuncInput::String,
            extra_args: &[ExtraArg::IntLiteral("1"), ExtraArg::IntLiteral("3")],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "SPLIT_PART",
            input: FuncInput::String,
            extra_args: &[ExtraArg::StringLiteral(","), ExtraArg::IntLiteral("1")],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "STRPOS",
            input: FuncInput::String,
            extra_args: &[ExtraArg::StringLiteral("l")],
            prepend_literal: None,
            output_type: DataType::BigInt,
        },
        // Temporal functions (literal-first argument order)
        FuncDesc {
            name: "DATE_PART",
            input: FuncInput::Temporal,
            extra_args: &[],
            prepend_literal: Some("year"),
            output_type: DataType::BigInt,
        },
        FuncDesc {
            name: "DATE_TRUNC",
            input: FuncInput::Temporal,
            extra_args: &[],
            prepend_literal: Some("month"),
            output_type: DataType::Timestamp {
                with_timezone: false,
            },
        },
        // JSON functions (using DuckDB-compatible names for testing)
        FuncDesc {
            name: "TO_JSON",
            input: FuncInput::AnyScalar,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "JSON_ARRAY",
            input: FuncInput::AnyScalar,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        FuncDesc {
            name: "JSON_OBJECT",
            input: FuncInput::AnyScalar,
            extra_args: &[],
            prepend_literal: Some("key"),
            output_type: DataType::Text,
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
        FuncInput::BooleanAggregate => base == BaseType::Boolean,
        FuncInput::IntegerAggregate => {
            matches!(base, BaseType::Integer | BaseType::BigInt)
        }
        FuncInput::NoArg => true,
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
        "BOOL_AND" | "BOOL_OR" | "EVERY" => DataType::Boolean,
        "BIT_AND" | "BIT_OR" | "BIT_XOR" => arg_type.clone(),
        "LENGTH" | "CHAR_LENGTH" | "CHARACTER_LENGTH" | "STRPOS" | "POSITION" | "DATE_PART" => {
            DataType::BigInt
        }
        "DATE_TRUNC" => DataType::Timestamp {
            with_timezone: false,
        },
        "SQRT" | "EXP" | "LN" | "LOG" | "LOG10" | "LOG2" | "POWER" | "POW" | "SIN" | "COS"
        | "TAN" | "ASIN" | "ACOS" | "ATAN" | "ATAN2" | "SINH" | "COSH" | "TANH" | "PI" => {
            DataType::Double
        }
        "MOD" => arg_type.clone(),
        // String functions
        "UPPER" | "LOWER" | "TRIM" | "LTRIM" | "RTRIM" | "REVERSE" | "CONCAT" | "REPLACE"
        | "REPEAT" | "LPAD" | "RPAD" | "INITCAP" | "SUBSTRING" | "SUBSTR" | "LEFT" | "RIGHT"
        | "SPLIT_PART" => DataType::Text,
        // JSON functions
        "TO_JSON"
        | "JSON_OBJECT"
        | "JSON_ARRAY"
        | "JSON_EXTRACT"
        | "JSON_EXTRACT_STRING"
        | "JSON_EXTRACT_TEXT" => DataType::Text,
        "JSON_ARRAY_LENGTH" => DataType::BigInt,
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
        1 => Just(ExprKind::Between),
        1 => Just(ExprKind::InList),
        1 => Just(ExprKind::JsonOp),
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
            let col = &columns[expr_idx % columns.len()];
            // Pick a cast target based on func_idx to get variety
            let cast_options: &[(&str, DataType)] = if col.data_type.is_numeric() {
                &[
                    ("DOUBLE", DataType::Double),
                    ("INTEGER", DataType::Integer),
                    ("BIGINT", DataType::BigInt),
                    ("VARCHAR", DataType::Varchar { max_length: None }),
                ]
            } else if col.data_type.is_string() {
                &[("VARCHAR", DataType::Varchar { max_length: None })]
            } else if matches!(col.data_type, DataType::Date) {
                &[
                    (
                        "TIMESTAMP",
                        DataType::Timestamp {
                            with_timezone: false,
                        },
                    ),
                    ("VARCHAR", DataType::Varchar { max_length: None }),
                ]
            } else if matches!(col.data_type, DataType::Timestamp { .. }) {
                &[
                    ("DATE", DataType::Date),
                    ("VARCHAR", DataType::Varchar { max_length: None }),
                ]
            } else {
                &[("VARCHAR", DataType::Varchar { max_length: None })]
            };
            let (cast_type, smelt_type) = &cast_options[func_idx % cast_options.len()];
            Some(TypedExpr {
                sql: format!("CAST({} AS {cast_type})", col.name),
                alias,
                expected_smelt_type: smelt_type.clone(),
            })
        }

        ExprKind::Function => {
            let funcs = core_functions();
            let func = &funcs[func_idx % funcs.len()];

            // Handle zero-arg functions
            if matches!(func.input, FuncInput::NoArg) {
                return Some(TypedExpr {
                    sql: format!("{}()", func.name),
                    alias,
                    expected_smelt_type: func.output_type.clone(),
                });
            }

            // Find a compatible column
            let compatible_col = columns.iter().find(|c| {
                smelt_type_to_base(&c.data_type).is_some_and(|b| is_compatible(b, func.input))
            })?;

            let return_type = function_return_type(func.name, &compatible_col.data_type);

            // Build argument list
            let mut args = Vec::new();
            if let Some(lit) = func.prepend_literal {
                args.push(format!("'{lit}'"));
            }
            args.push(compatible_col.name.clone());
            for extra in func.extra_args {
                match extra {
                    ExtraArg::SameAsFirst => args.push(compatible_col.name.clone()),
                    ExtraArg::IntLiteral(v) => args.push(v.to_string()),
                    ExtraArg::StringLiteral(v) => args.push(format!("'{v}'")),
                }
            }
            let sql = format!("{}({})", func.name, args.join(", "));

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

        ExprKind::Between => {
            // Find a numeric column for BETWEEN
            let num_col = columns.iter().find(|c| c.data_type.is_numeric())?;
            Some(TypedExpr {
                sql: format!("{} BETWEEN 0 AND 100", num_col.name),
                alias,
                expected_smelt_type: DataType::Boolean,
            })
        }

        ExprKind::InList => {
            // Find a numeric column for IN
            let num_col = columns.iter().find(|c| c.data_type.is_numeric())?;
            Some(TypedExpr {
                sql: format!("{} IN (1, 2, 3)", num_col.name),
                alias,
                expected_smelt_type: DataType::Boolean,
            })
        }

        ExprKind::JsonOp => {
            // Generate JSON -> or ->> operator expressions
            // Use a JSON literal and pick an operator based on func_idx
            let json_literal = r#"CAST('{"a":1,"b":"hello","c":true}' AS JSON)"#;
            let keys = ["a", "b", "c"];
            let key = keys[expr_idx % keys.len()];
            let op = if func_idx.is_multiple_of(2) {
                "->"
            } else {
                "->>"
            };
            Some(TypedExpr {
                sql: format!("{json_literal} {op} '{key}'"),
                alias,
                expected_smelt_type: DataType::Text,
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
