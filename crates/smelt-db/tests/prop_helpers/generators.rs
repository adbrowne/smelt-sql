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
    TimestampTz,
    Decimal,
    Time,
    Interval,
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
            BaseType::TimestampTz,
            BaseType::Decimal,
            BaseType::Time,
            BaseType::Interval,
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
            BaseType::TimestampTz => DataType::Timestamp {
                with_timezone: true,
            },
            BaseType::Decimal => DataType::Decimal {
                precision: 10,
                scale: 2,
            },
            BaseType::Time => DataType::Time,
            BaseType::Interval => DataType::Interval,
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
            BaseType::TimestampTz => "CAST('2024-01-01 12:00:00+00' AS TIMESTAMPTZ)",
            BaseType::Decimal => "CAST(99.99 AS DECIMAL(10,2))",
            BaseType::Time => "CAST('12:00:00' AS TIME)",
            BaseType::Interval => "CAST('1 day' AS INTERVAL)",
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
            BaseType::TimestampTz => "tstz_col",
            BaseType::Decimal => "dec_col",
            BaseType::Time => "time_col",
            BaseType::Interval => "interval_col",
        }
    }
}

/// The shape of the generated query — controls GROUP BY and HAVING generation.
#[derive(Debug, Clone)]
pub enum QueryShape {
    /// `SELECT scalar_exprs FROM data`
    Scalar,
    /// `SELECT group_cols, agg_exprs FROM data GROUP BY group_cols`
    GroupBy { group_columns: Vec<String> },
    /// `SELECT group_cols, agg_exprs FROM data GROUP BY group_cols HAVING predicate`
    GroupByHaving {
        group_columns: Vec<String>,
        having_predicate: String,
    },
    /// `SELECT window_exprs FROM data`
    Window,
    /// `SELECT group_cols, agg_exprs, window_over_agg FROM data GROUP BY group_cols`
    GroupByWindow { group_columns: Vec<String> },
    /// `SELECT DISTINCT exprs FROM data`
    Distinct,
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
    /// Window function with OVER clause.
    WindowFunc,
    /// JSON operator (-> or ->>).
    JsonOp,
    /// col IS NULL / col IS NOT NULL → Boolean.
    IsNull,
    /// Comparison operators (=, <>, <, >, <=, >=) → Boolean.
    Comparison,
    /// NOT bool_col → Boolean.
    UnaryNot,
    /// -num_col → same numeric type.
    UnaryMinus,
    /// EXISTS (SELECT 1 FROM data WHERE bool_col) → Boolean.
    Exists,
    /// EXTRACT(part FROM temporal_col) → BigInt.
    Extract,
    /// MAKE_DATE(y, m, d) or MAKE_TIMESTAMP(y, m, d, h, min, s).
    MakeTemporal,
    /// (SELECT agg(col) FROM data) → aggregate return type.
    ScalarSubquery,
    /// LIKE / ILIKE pattern matching → Boolean.
    Like,
    /// Regex operators (~, ~*) → Boolean.
    Regex,
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
            output_type: DataType::unknown_dynamic(), // arg-dependent
        },
        FuncDesc {
            name: "MAX",
            input: FuncInput::AnyAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::unknown_dynamic(), // arg-dependent
        },
        // Null-handling / scalar functions that accept any type
        FuncDesc {
            name: "COALESCE",
            input: FuncInput::AnyScalar,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::unknown_dynamic(), // arg-dependent
        },
        // Comparison functions
        FuncDesc {
            name: "GREATEST",
            input: FuncInput::AnyScalar,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::unknown_dynamic(), // arg-dependent
        },
        FuncDesc {
            name: "LEAST",
            input: FuncInput::AnyScalar,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::unknown_dynamic(), // arg-dependent
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
            output_type: DataType::unknown_dynamic(), // arg-dependent
        },
        FuncDesc {
            name: "BIT_OR",
            input: FuncInput::IntegerAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::unknown_dynamic(), // arg-dependent
        },
        FuncDesc {
            name: "BIT_XOR",
            input: FuncInput::IntegerAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::unknown_dynamic(), // arg-dependent
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
            output_type: DataType::unknown_dynamic(), // arg-dependent
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
            output_type: DataType::unknown_dynamic(), // arg-dependent
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
        // String aggregate
        FuncDesc {
            name: "STRING_AGG",
            input: FuncInput::String,
            extra_args: &[ExtraArg::StringLiteral(",")],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        // Any-type aggregates
        FuncDesc {
            name: "ANY_VALUE",
            input: FuncInput::AnyAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::unknown_dynamic(), // uses input type
        },
        FuncDesc {
            name: "APPROX_COUNT_DISTINCT",
            input: FuncInput::AnyAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::BigInt,
        },
        // ---- Extended coverage: temporal/statistical/JSON/string functions ----
        // MEDIAN(integer) interpolates to Double in DuckDB/Spark; Decimal/Double
        // preserve their type (smelt fix: median_integer_infers_double).
        FuncDesc {
            name: "MEDIAN",
            input: FuncInput::NumericAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        // ARRAY_AGG(col) → Array(col_type); DuckDB returns T[] (Arrow List).
        FuncDesc {
            name: "ARRAY_AGG",
            input: FuncInput::AnyAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::unknown_dynamic(), // arg-dependent
        },
        // AGE(temporal) → Interval.
        FuncDesc {
            name: "AGE",
            input: FuncInput::Temporal,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::Interval,
        },
        // JSON_EXTRACT(json_text, path) → JSON (Arrow Varchar); smelt models JSON
        // as Text (Text/Varchar compatible).
        FuncDesc {
            name: "JSON_EXTRACT",
            input: FuncInput::String,
            extra_args: &[ExtraArg::StringLiteral("$.a")],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        // IFNULL(a, b) ≡ COALESCE(a, b) → argument type.
        FuncDesc {
            name: "IFNULL",
            input: FuncInput::Numeric,
            extra_args: &[ExtraArg::IntLiteral("0")],
            prepend_literal: None,
            output_type: DataType::unknown_dynamic(), // arg-dependent
        },
        // TRANSLATE(s, from, to) → Varchar (smelt Text; compatible).
        FuncDesc {
            name: "TRANSLATE",
            input: FuncInput::String,
            extra_args: &[ExtraArg::StringLiteral("a"), ExtraArg::StringLiteral("b")],
            prepend_literal: None,
            output_type: DataType::Text,
        },
        // Two-argument statistical aggregates → Double (DuckDB and smelt agree).
        FuncDesc {
            name: "CORR",
            input: FuncInput::NumericAggregate,
            extra_args: &[ExtraArg::SameAsFirst],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "COVAR_POP",
            input: FuncInput::NumericAggregate,
            extra_args: &[ExtraArg::SameAsFirst],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        FuncDesc {
            name: "COVAR_SAMP",
            input: FuncInput::NumericAggregate,
            extra_args: &[ExtraArg::SameAsFirst],
            prepend_literal: None,
            output_type: DataType::Double,
        },
        // MODE(col) → the column's type (DuckDB and smelt agree).
        FuncDesc {
            name: "MODE",
            input: FuncInput::AnyAggregate,
            extra_args: &[],
            prepend_literal: None,
            output_type: DataType::unknown_dynamic(), // arg-dependent
        },
        // NOTE: INITCAP and TO_CHAR are not DuckDB scalar functions, and the
        // SQL-standard `POSITION(x IN y)` / `PERCENTILE_CONT(f) WITHIN GROUP`
        // forms are deferred parser grammar. They are intentionally not
        // generated here — the differential-parsing gaps track them instead.
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
        FuncInput::Temporal => matches!(
            base,
            BaseType::Date | BaseType::Timestamp | BaseType::TimestampTz
        ),
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
        "MIN" | "MAX" | "COALESCE" | "NULLIF" | "IFNULL" | "GREATEST" | "LEAST" | "ANY_VALUE" => {
            arg_type.clone()
        }
        // MEDIAN interpolates integer inputs to Double; other numerics keep type.
        "MEDIAN" => match arg_type {
            DataType::SmallInt | DataType::Integer | DataType::BigInt => DataType::Double,
            other => other.clone(),
        },
        "PERCENTILE_CONT" | "PERCENTILE_DISC" | "CORR" | "COVAR_POP" | "COVAR_SAMP" => {
            DataType::Double
        }
        "ARRAY_AGG" => DataType::Array(Box::new(arg_type.clone())),
        "AGE" => DataType::Interval,
        "MODE" => arg_type.clone(),
        "COUNT" | "APPROX_COUNT_DISTINCT" => DataType::BigInt,
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
        | "REPEAT" | "LPAD" | "RPAD" | "INITCAP" | "TRANSLATE" | "TO_CHAR" | "SUBSTRING"
        | "SUBSTR" | "LEFT" | "RIGHT" | "SPLIT_PART" | "STRING_AGG" => DataType::Text,
        // JSON functions
        "TO_JSON"
        | "JSON_OBJECT"
        | "JSON_ARRAY"
        | "JSON_EXTRACT"
        | "JSON_EXTRACT_STRING"
        | "JSON_EXTRACT_TEXT" => DataType::Text,
        "JSON_ARRAY_LENGTH" => DataType::BigInt,
        _ => DataType::unknown_dynamic(),
    }
}

// ---- Proptest strategies ----

/// Strategy that picks a random BaseType.
pub fn base_type_strategy() -> impl Strategy<Value = BaseType> {
    prop::sample::select(BaseType::all())
}

/// Strategy that generates a pool of 1-5 typed columns.
///
/// With a 25% "tz-mixing" weight the pool is augmented with both a naive
/// `Timestamp` and a `Timestamp WITH TIME ZONE` column, so the generators can
/// reach mixed naive/tz-aware timestamp expressions (comparisons here; the
/// tz-mixed *arithmetic* diagnostic path in the inference) that a uniform draw
/// would produce only rarely.
pub fn column_pool_strategy() -> impl Strategy<Value = Vec<TypedSource>> {
    (
        prop::collection::vec(base_type_strategy(), 1..=5),
        proptest::bool::weighted(0.25),
    )
        .prop_map(|(mut types, tz_mix)| {
            if tz_mix {
                types.push(BaseType::Timestamp);
                types.push(BaseType::TimestampTz);
            }
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
    // `Extract` and `MakeTemporal` were previously excluded due to a documented
    // (but stale) parser/harness issue around the `FROM` keyword inside
    // `EXTRACT(part FROM expr)`. Phase 58 (April 2026) verified the bug had
    // already been fixed end-to-end and re-enabled both entries here.
    prop_oneof![
        2 => Just(ExprKind::ColumnRef),
        3 => Just(ExprKind::Function),
        2 => Just(ExprKind::BinaryOp),
        1 => Just(ExprKind::CaseExpr),
        1 => Just(ExprKind::Cast),
        1 => Just(ExprKind::Between),
        1 => Just(ExprKind::InList),
        2 => Just(ExprKind::WindowFunc),
        1 => Just(ExprKind::JsonOp),
        1 => Just(ExprKind::IsNull),
        1 => Just(ExprKind::Comparison),
        1 => Just(ExprKind::UnaryNot),
        1 => Just(ExprKind::UnaryMinus),
        1 => Just(ExprKind::Exists),
        1 => Just(ExprKind::ScalarSubquery),
        1 => Just(ExprKind::Like),
        1 => Just(ExprKind::Regex),
        1 => Just(ExprKind::Extract),
        1 => Just(ExprKind::MakeTemporal),
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
                    (
                        "DECIMAL(12,3)",
                        DataType::Decimal {
                            precision: 12,
                            scale: 3,
                        },
                    ),
                    // smelt normalises FLOAT → Double (registered divergence
                    // `cast_float_as_double`); DuckDB reports FLOAT.
                    ("FLOAT", DataType::Double),
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
            // Alternate between CAST() and :: syntax
            let sql = if expr_idx.is_multiple_of(2) {
                format!("CAST({} AS {cast_type})", col.name)
            } else {
                format!("{}::{cast_type}", col.name)
            };
            Some(TypedExpr {
                sql,
                alias,
                expected_smelt_type: smelt_type.clone(),
            })
        }

        ExprKind::Function => {
            let funcs = core_functions();
            // Spread the selection across the whole list: `func_idx` ranges only
            // 0..100 while the list is longer, so a bare modulo leaves the tail
            // (where the extended functions live) reachable by a single index.
            // Mixing in `expr_idx` with a coprime multiplier gives every entry a
            // fair share.
            let sel = func_idx
                .wrapping_mul(7)
                .wrapping_add(expr_idx.wrapping_mul(3));
            let func = &funcs[sel % funcs.len()];

            // JSON_EXTRACT needs a *valid JSON* first argument: the oracle
            // executes the query, and feeding it an arbitrary string column
            // (e.g. 'hello') raises a runtime "Malformed JSON" error. Use a JSON
            // literal so the path is exercised on well-formed input.
            if func.name == "JSON_EXTRACT" {
                return Some(TypedExpr {
                    sql: r#"JSON_EXTRACT('{"a":1}', '$.a')"#.to_string(),
                    alias,
                    expected_smelt_type: DataType::Text,
                });
            }

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

        ExprKind::BinaryOp => generate_binary_op_expr(columns, expr_idx, func_idx, alias),

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

        ExprKind::WindowFunc => generate_window_expr(columns, expr_idx, func_idx, alias),

        ExprKind::IsNull => {
            let col = &columns[expr_idx % columns.len()];
            let is_not = func_idx.is_multiple_of(2);
            let op = if is_not { "IS NOT NULL" } else { "IS NULL" };
            Some(TypedExpr {
                sql: format!("{} {op}", col.name),
                alias,
                expected_smelt_type: DataType::Boolean,
            })
        }

        ExprKind::Comparison => {
            let ops = ["=", "<>", "<", ">", "<=", ">="];
            let op = ops[func_idx % ops.len()];
            // Mixed naive/tz-aware timestamp comparison: DuckDB and smelt both
            // yield Boolean (comparison aligns the tz axis; only tz-mixed
            // *arithmetic* is an error, spec §16). Exercising this keeps the
            // tz-mixing path covered without tripping the arithmetic diagnostic.
            let naive_ts = columns.iter().find(|c| {
                matches!(
                    c.data_type,
                    DataType::Timestamp {
                        with_timezone: false
                    }
                )
            });
            let tz_ts = columns.iter().find(|c| {
                matches!(
                    c.data_type,
                    DataType::Timestamp {
                        with_timezone: true
                    }
                )
            });
            if let (Some(a), Some(b)) = (naive_ts, tz_ts) {
                if func_idx.is_multiple_of(2) {
                    return Some(TypedExpr {
                        sql: format!("{} {op} {}", a.name, b.name),
                        alias,
                        expected_smelt_type: DataType::Boolean,
                    });
                }
            }
            // Find two columns of the same type for comparison
            let col = &columns[expr_idx % columns.len()];
            Some(TypedExpr {
                sql: format!("{} {op} {}", col.name, col.name),
                alias,
                expected_smelt_type: DataType::Boolean,
            })
        }

        ExprKind::UnaryNot => {
            // Find a boolean column, or wrap a comparison
            if let Some(bool_col) = columns.iter().find(|c| c.data_type == DataType::Boolean) {
                Some(TypedExpr {
                    sql: format!("NOT {}", bool_col.name),
                    alias,
                    expected_smelt_type: DataType::Boolean,
                })
            } else {
                // Generate NOT (col = col) as fallback
                let col = &columns[expr_idx % columns.len()];
                Some(TypedExpr {
                    sql: format!("NOT ({} = {})", col.name, col.name),
                    alias,
                    expected_smelt_type: DataType::Boolean,
                })
            }
        }

        ExprKind::UnaryMinus => {
            let num_col = columns.iter().find(|c| c.data_type.is_numeric())?;
            Some(TypedExpr {
                sql: format!("-{}", num_col.name),
                alias,
                expected_smelt_type: num_col.data_type.clone(),
            })
        }

        ExprKind::Exists => {
            // EXISTS (SELECT 1 FROM data WHERE col IS NOT NULL)
            let col = &columns[expr_idx % columns.len()];
            Some(TypedExpr {
                sql: format!("EXISTS (SELECT 1 FROM data WHERE {} IS NOT NULL)", col.name),
                alias,
                expected_smelt_type: DataType::Boolean,
            })
        }

        ExprKind::Extract => {
            // EXTRACT(part FROM temporal_col) → BigInt
            let temporal_col = columns
                .iter()
                .find(|c| matches!(c.data_type, DataType::Date | DataType::Timestamp { .. }))?;
            // EPOCH returns fractional seconds → Double; calendar fields → BigInt.
            let parts = ["YEAR", "MONTH", "DAY", "EPOCH"];
            let part = parts[func_idx % parts.len()];
            let expected = if part == "EPOCH" {
                DataType::Double
            } else {
                DataType::BigInt
            };
            Some(TypedExpr {
                sql: format!("EXTRACT({part} FROM {})", temporal_col.name),
                alias,
                expected_smelt_type: expected,
            })
        }

        ExprKind::Like => {
            let str_col = columns.iter().find(|c| c.data_type.is_string())?;
            let op = if func_idx.is_multiple_of(2) {
                "LIKE"
            } else {
                "ILIKE"
            };
            Some(TypedExpr {
                sql: format!("{} {op} '%ell%'", str_col.name),
                alias,
                expected_smelt_type: DataType::Boolean,
            })
        }

        ExprKind::Regex => {
            // Use ~ only (DuckDB doesn't support ~*, !~, !~*)
            let str_col = columns.iter().find(|c| c.data_type.is_string())?;
            Some(TypedExpr {
                sql: format!("{} ~ 'ell'", str_col.name),
                alias,
                expected_smelt_type: DataType::Boolean,
            })
        }

        ExprKind::ScalarSubquery => {
            // (SELECT COUNT(*) FROM data) or (SELECT MIN(col) FROM data)
            let col = &columns[expr_idx % columns.len()];
            if func_idx.is_multiple_of(2) {
                Some(TypedExpr {
                    sql: "(SELECT COUNT(*) FROM data)".to_string(),
                    alias,
                    expected_smelt_type: DataType::BigInt,
                })
            } else {
                Some(TypedExpr {
                    sql: format!("(SELECT MIN({}) FROM data)", col.name),
                    alias,
                    expected_smelt_type: col.data_type.clone(),
                })
            }
        }

        ExprKind::MakeTemporal => {
            // MAKE_DATE or MAKE_TIMESTAMP from integer columns
            // Use column as year (any integer is a valid year), safe literals for month/day
            let int_col = columns
                .iter()
                .find(|c| matches!(c.data_type, DataType::Integer | DataType::BigInt))?;
            if func_idx.is_multiple_of(2) {
                Some(TypedExpr {
                    sql: format!("MAKE_DATE({}, 1, 1)", int_col.name),
                    alias,
                    expected_smelt_type: DataType::Date,
                })
            } else {
                Some(TypedExpr {
                    sql: format!("MAKE_TIMESTAMP({}, 1, 1, 0, 0, 0)", int_col.name),
                    alias,
                    expected_smelt_type: DataType::Timestamp {
                        with_timezone: false,
                    },
                })
            }
        }
    }
}

/// Generate a binary-operation expression, covering numeric, string,
/// decimal-arithmetic, and temporal-arithmetic paths.
///
/// The expected-type computation mirrors the SPEC formulas (types.md §15
/// decimal growth and §16 temporal/tz-transparent arithmetic); it does not
/// import production inference. Each satisfiable operand pairing contributes a
/// candidate; one is selected deterministically by `(expr_idx, func_idx)` so
/// the corpus reaches every path over enough cases.
fn generate_binary_op_expr(
    columns: &[TypedSource],
    expr_idx: usize,
    func_idx: usize,
    alias: String,
) -> Option<TypedExpr> {
    let by = |pred: &dyn Fn(&DataType) -> bool| -> Vec<&TypedSource> {
        columns.iter().filter(|c| pred(&c.data_type)).collect()
    };

    let interval_cols = by(&|d| matches!(d, DataType::Interval));
    let date_cols = by(&|d| matches!(d, DataType::Date));
    let ts_cols = by(&|d| {
        matches!(
            d,
            DataType::Timestamp {
                with_timezone: false
            }
        )
    });
    let tstz_cols = by(&|d| {
        matches!(
            d,
            DataType::Timestamp {
                with_timezone: true
            }
        )
    });
    let time_cols = by(&|d| matches!(d, DataType::Time));
    let dec_cols = by(&|d| matches!(d, DataType::Decimal { .. }));
    // Non-decimal numeric operands keep "/" generation portable (Decimal / T is
    // rejected by spec §15, covered separately via the decimal candidates).
    let num_cols = by(&|d| d.is_numeric() && !matches!(d, DataType::Decimal { .. }));
    let str_cols = by(&|d| d.is_string());

    // (sql, expected_smelt_type) candidates.
    let mut candidates: Vec<(String, DataType)> = Vec::new();

    // ---- Temporal arithmetic (spec §16: tz-transparent) ----
    if let (Some(iv), Some(d)) = (interval_cols.first(), date_cols.first()) {
        // DATE ± INTERVAL → Timestamp (matches DuckDB). Spark instead returns
        // DATE for a day/year-month-granularity interval (only TIMESTAMP once
        // the interval carries a time component) — see the `date_plus_interval`
        // divergence entry in divergences.rs. The generator only ever emits a
        // day-granularity interval literal, so this expectation matches smelt's
        // own (DuckDB-aligned) inference, not what Spark actually returns.
        candidates.push((
            format!("{} + {}", d.name, iv.name),
            DataType::Timestamp {
                with_timezone: false,
            },
        ));
        candidates.push((
            format!("{} - {}", d.name, iv.name),
            DataType::Timestamp {
                with_timezone: false,
            },
        ));
    }
    if let (Some(iv), Some(t)) = (interval_cols.first(), ts_cols.first()) {
        candidates.push((
            format!("{} + {}", t.name, iv.name),
            DataType::Timestamp {
                with_timezone: false,
            },
        ));
        candidates.push((
            format!("{} - {}", t.name, iv.name),
            DataType::Timestamp {
                with_timezone: false,
            },
        ));
    }
    if let (Some(iv), Some(t)) = (interval_cols.first(), tstz_cols.first()) {
        candidates.push((
            format!("{} + {}", t.name, iv.name),
            DataType::Timestamp {
                with_timezone: true,
            },
        ));
        candidates.push((
            format!("{} - {}", t.name, iv.name),
            DataType::Timestamp {
                with_timezone: true,
            },
        ));
    }
    if let (Some(iv), Some(t)) = (interval_cols.first(), time_cols.first()) {
        candidates.push((format!("{} + {}", t.name, iv.name), DataType::Time));
    }
    if let Some(iv) = interval_cols.first() {
        // INTERVAL ± INTERVAL → Interval.
        candidates.push((format!("{} + {}", iv.name, iv.name), DataType::Interval));
        candidates.push((format!("{} - {}", iv.name, iv.name), DataType::Interval));
    }
    // Temporal differences → Interval (smelt, Spark-aligned).
    //
    // NOTE: `DATE - DATE` is intentionally NOT generated. smelt infers Interval
    // (docs/type_semantics.md), but DuckDB returns BIGINT (a day count) that it
    // cannot cast back to INTERVAL — so the cast-wrap conformance oracle rejects
    // it. It is a registered divergence (`date_minus_date`); the Interval-typed
    // temporal-difference path is covered here by TIMESTAMP/TIMESTAMPTZ
    // subtraction, which DuckDB also types as INTERVAL.
    if let Some(t) = ts_cols.first() {
        candidates.push((format!("{} - {}", t.name, t.name), DataType::Interval));
    }
    if let Some(t) = tstz_cols.first() {
        candidates.push((format!("{} - {}", t.name, t.name), DataType::Interval));
    }
    // NOTE: `TIME - TIME` is intentionally omitted — DuckDB has no `-(TIME, TIME)`
    // overload, so it is not in the portable surface (`TIME - INTERVAL` is).

    // ---- Decimal arithmetic (spec §15 growth formulas) ----
    if let Some(a) = dec_cols.first() {
        let b = dec_cols.get(1).copied().unwrap_or(a);
        let (pa, sa) = decimal_ps(&a.data_type);
        let (pb, sb) = decimal_ps(&b.data_type);
        candidates.push((
            format!("{} + {}", a.name, b.name),
            decimal_add_type(pa, sa, pb, sb),
        ));
        candidates.push((
            format!("{} * {}", a.name, b.name),
            decimal_mul_type(pa, sa, pb, sb),
        ));
        // Decimal / Decimal is rejected by smelt (spec §15) → Unknown; still
        // generated so the property test exercises the rejection path.
        candidates.push((
            format!("{} / {}", a.name, b.name),
            DataType::unknown_dynamic(),
        ));
    }

    // ---- Numeric arithmetic (non-decimal) ----
    let num_ops = ["+", "-", "*", "%", "/"];
    if num_cols.len() >= 2 {
        let a = num_cols[expr_idx % num_cols.len()];
        let b = num_cols[(expr_idx + 1) % num_cols.len()];
        for op in num_ops {
            let expected = if op == "/" {
                DataType::Double
            } else {
                promote_numeric_type(&a.data_type, &b.data_type)
            };
            candidates.push((format!("{} {} {}", a.name, op, b.name), expected));
        }
    } else if let Some(n) = num_cols.first() {
        for op in num_ops {
            let expected = if op == "/" {
                DataType::Double
            } else {
                n.data_type.clone()
            };
            candidates.push((format!("{} {} {}", n.name, op, n.name), expected));
        }
    }

    // ---- String concatenation ----
    if let Some(s) = str_cols.first() {
        candidates.push((format!("{} || {}", s.name, s.name), DataType::Text));
    }

    if candidates.is_empty() {
        return None;
    }
    let (sql, expected_smelt_type) =
        candidates[(expr_idx.wrapping_mul(7).wrapping_add(func_idx)) % candidates.len()].clone();
    Some(TypedExpr {
        sql,
        alias,
        expected_smelt_type,
    })
}

/// Extract (precision, scale) from a Decimal type; falls back to (10, 2).
fn decimal_ps(dt: &DataType) -> (u8, u8) {
    match dt {
        DataType::Decimal { precision, scale } => (*precision, *scale),
        _ => (10, 2),
    }
}

/// Spec §15 additive growth: p' = max(p1-s1, p2-s2) + max(s1,s2) + 1, s' = max(s1,s2).
/// Overflow (p' > 38) collapses to Unknown, mirroring the inference contract.
fn decimal_add_type(p1: u8, s1: u8, p2: u8, s2: u8) -> DataType {
    let s_prime = s1.max(s2) as u32;
    let p_prime = (p1.saturating_sub(s1)).max(p2.saturating_sub(s2)) as u32 + s_prime + 1;
    if p_prime > 38 {
        DataType::unknown_dynamic()
    } else {
        DataType::Decimal {
            precision: p_prime as u8,
            scale: s_prime as u8,
        }
    }
}

/// Spec §15 multiplicative growth: p' = p1 + p2 + 1, s' = s1 + s2.
fn decimal_mul_type(p1: u8, s1: u8, p2: u8, s2: u8) -> DataType {
    let p_prime = p1 as u32 + p2 as u32 + 1;
    let s_prime = s1 as u32 + s2 as u32;
    if p_prime > 38 {
        DataType::unknown_dynamic()
    } else {
        DataType::Decimal {
            precision: p_prime as u8,
            scale: s_prime as u8,
        }
    }
}

/// Window function descriptors organized by category.
#[derive(Debug, Clone, Copy)]
enum WindowFuncKind {
    /// No arguments, returns BigInt (ROW_NUMBER, RANK, DENSE_RANK).
    RankingBigInt,
    /// Takes an integer argument, returns BigInt (NTILE).
    NtileBigInt,
    /// No arguments, returns Double (CUME_DIST, PERCENT_RANK).
    RankingDouble,
    /// Takes a column argument, returns the column's type (LAG, LEAD, FIRST_VALUE, LAST_VALUE).
    ValueFunc,
    /// Takes a column + integer argument, returns the column's type (NTH_VALUE).
    NthValueFunc,
}

struct WindowFuncDesc {
    name: &'static str,
    kind: WindowFuncKind,
    /// Whether ORDER BY is required for correct semantics.
    requires_order_by: bool,
}

const WINDOW_FUNCTIONS: &[WindowFuncDesc] = &[
    WindowFuncDesc {
        name: "ROW_NUMBER",
        kind: WindowFuncKind::RankingBigInt,
        requires_order_by: true,
    },
    WindowFuncDesc {
        name: "RANK",
        kind: WindowFuncKind::RankingBigInt,
        requires_order_by: true,
    },
    WindowFuncDesc {
        name: "DENSE_RANK",
        kind: WindowFuncKind::RankingBigInt,
        requires_order_by: true,
    },
    WindowFuncDesc {
        name: "NTILE",
        kind: WindowFuncKind::NtileBigInt,
        requires_order_by: true,
    },
    WindowFuncDesc {
        name: "CUME_DIST",
        kind: WindowFuncKind::RankingDouble,
        requires_order_by: true,
    },
    WindowFuncDesc {
        name: "PERCENT_RANK",
        kind: WindowFuncKind::RankingDouble,
        requires_order_by: true,
    },
    WindowFuncDesc {
        name: "LAG",
        kind: WindowFuncKind::ValueFunc,
        requires_order_by: true,
    },
    WindowFuncDesc {
        name: "LEAD",
        kind: WindowFuncKind::ValueFunc,
        requires_order_by: true,
    },
    WindowFuncDesc {
        name: "FIRST_VALUE",
        kind: WindowFuncKind::ValueFunc,
        requires_order_by: false,
    },
    WindowFuncDesc {
        name: "LAST_VALUE",
        kind: WindowFuncKind::ValueFunc,
        requires_order_by: false,
    },
    WindowFuncDesc {
        name: "NTH_VALUE",
        kind: WindowFuncKind::NthValueFunc,
        requires_order_by: false,
    },
];

/// Generate a window function expression with an OVER clause.
fn generate_window_expr(
    columns: &[TypedSource],
    expr_idx: usize,
    func_idx: usize,
    alias: String,
) -> Option<TypedExpr> {
    let wf = &WINDOW_FUNCTIONS[func_idx % WINDOW_FUNCTIONS.len()];

    // Pick columns for the value argument and OVER clause
    let col = &columns[expr_idx % columns.len()];
    let order_col = &columns[(expr_idx + 1) % columns.len()];
    // Use a different column for PARTITION BY when available
    let partition_col = if columns.len() > 2 {
        Some(&columns[(expr_idx + 2) % columns.len()])
    } else {
        None
    };

    // Build the function call
    let func_call = match wf.kind {
        WindowFuncKind::RankingBigInt | WindowFuncKind::RankingDouble => {
            format!("{}()", wf.name)
        }
        WindowFuncKind::NtileBigInt => {
            format!("{}(4)", wf.name)
        }
        WindowFuncKind::ValueFunc => {
            format!("{}({})", wf.name, col.name)
        }
        WindowFuncKind::NthValueFunc => {
            format!("{}({}, 1)", wf.name, col.name)
        }
    };

    // Build OVER clause
    let mut over_parts = Vec::new();
    if let Some(pcol) = partition_col {
        over_parts.push(format!("PARTITION BY {}", pcol.name));
    }
    if wf.requires_order_by || func_idx.is_multiple_of(2) {
        over_parts.push(format!("ORDER BY {}", order_col.name));

        // Add window frame spec sometimes (only valid with ORDER BY)
        // Skip for ranking functions (ROW_NUMBER, RANK, DENSE_RANK) which don't support frames
        if !matches!(wf.kind, WindowFuncKind::RankingBigInt) && expr_idx.is_multiple_of(3) {
            let frame_specs = [
                "ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW",
                "ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING",
                "ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING",
                "ROWS UNBOUNDED PRECEDING",
            ];
            over_parts.push(frame_specs[func_idx % frame_specs.len()].to_string());
        }
    }

    let over_clause = if over_parts.is_empty() {
        "OVER ()".to_string()
    } else {
        format!("OVER ({})", over_parts.join(" "))
    };

    // Determine return type
    let expected_type = match wf.kind {
        WindowFuncKind::RankingBigInt | WindowFuncKind::NtileBigInt => DataType::BigInt,
        WindowFuncKind::RankingDouble => DataType::Double,
        WindowFuncKind::ValueFunc | WindowFuncKind::NthValueFunc => col.data_type.clone(),
    };

    Some(TypedExpr {
        sql: format!("{func_call} {over_clause}"),
        alias,
        expected_smelt_type: expected_type,
    })
}

/// Promote two numeric types to the widest type, matching type_inference.rs rules.
/// Double > Decimal > BigInt > Integer > SmallInt
fn promote_numeric_type(a: &DataType, b: &DataType) -> DataType {
    if matches!(a, DataType::Double) || matches!(b, DataType::Double) {
        DataType::Double
    } else if matches!(a, DataType::Float) || matches!(b, DataType::Float) {
        DataType::Float
    } else if matches!(a, DataType::Decimal { .. }) || matches!(b, DataType::Decimal { .. }) {
        DataType::Decimal {
            precision: 38,
            scale: 10,
        }
    } else if matches!(a, DataType::BigInt) || matches!(b, DataType::BigInt) {
        DataType::BigInt
    } else if matches!(a, DataType::Integer) || matches!(b, DataType::Integer) {
        DataType::Integer
    } else {
        DataType::SmallInt
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
        DataType::Timestamp {
            with_timezone: true,
        } => Some(BaseType::TimestampTz),
        DataType::Timestamp {
            with_timezone: false,
        } => Some(BaseType::Timestamp),
        DataType::Decimal { .. } => Some(BaseType::Decimal),
        DataType::SmallInt => Some(BaseType::Integer),
        DataType::Time => Some(BaseType::Time),
        DataType::Interval => Some(BaseType::Interval),
        _ => None,
    }
}

/// Check if a SQL expression string is an aggregate function call.
///
/// This is the single source of truth for aggregate detection.  `null_data.rs`
/// imports and reuses this function so both builders always agree on which
/// expressions are aggregates — eliminating the positional-alignment hazard that
/// arises when the two builders produce SELECT lists of different lengths.
pub(crate) fn is_aggregate_expr(sql: &str) -> bool {
    let upper = sql.to_uppercase();
    if let Some(paren_pos) = upper.find('(') {
        let name = upper[..paren_pos].trim();
        SqlFunction::from_name(name).is_some_and(|f| f.is_aggregate())
    } else {
        false
    }
}

/// Check if a SQL expression string is a window function call (contains OVER).
///
/// This is the single source of truth for window-function detection.  `null_data.rs`
/// imports and reuses this function so both builders always agree.
pub(crate) fn is_window_expr(sql: &str) -> bool {
    sql.to_uppercase().contains(" OVER ")
}

/// Assemble a CTE query from columns, expressions, and a query shape.
///
/// Handles three expression categories that cannot be freely mixed:
/// - Aggregates: require GROUP BY or single-row output
/// - Window functions: valid without GROUP BY, but conflict with bare aggregates
/// - Scalar expressions: column refs, casts, binary ops, etc.
///
/// When `QueryShape::GroupBy` or `GroupByHaving`, the SELECT list includes
/// group columns and aggregate expressions, with appropriate GROUP BY / HAVING clauses.
pub fn assemble_cte_query(
    columns: &[TypedSource],
    exprs: &[TypedExpr],
    shape: &QueryShape,
) -> String {
    // Build the CTE
    let cte_cols: Vec<String> = columns
        .iter()
        .map(|c| format!("{} AS {}", c.cast_sql, c.name))
        .collect();

    match shape {
        QueryShape::GroupBy { group_columns }
        | QueryShape::GroupByHaving { group_columns, .. }
        | QueryShape::GroupByWindow { group_columns } => {
            // For GROUP BY queries: include group columns + aggregate expressions only
            let mut select_items: Vec<String> = Vec::new();

            // Add group columns to SELECT
            for (i, gc) in group_columns.iter().enumerate() {
                select_items.push(format!("{gc} AS grp_{i}"));
            }

            // Filter to only aggregate expressions
            let agg_exprs: Vec<&TypedExpr> =
                exprs.iter().filter(|e| is_aggregate_expr(&e.sql)).collect();

            if agg_exprs.is_empty() {
                // No aggregates generated — use COUNT(*) as fallback
                select_items.push("COUNT(*) AS expr_0".to_string());
            } else {
                for e in &agg_exprs {
                    select_items.push(format!("{} AS {}", e.sql, e.alias));
                }
            }

            // For GroupByWindow, add a window function over an aggregate
            if matches!(shape, QueryShape::GroupByWindow { .. }) {
                // Pick an aggregate column to order by — use first agg or COUNT(*)
                let order_expr = if !agg_exprs.is_empty() {
                    agg_exprs[0].sql.clone()
                } else {
                    "COUNT(*)".to_string()
                };
                select_items.push(format!("RANK() OVER (ORDER BY {order_expr}) AS win_rank"));
            }

            let group_by_clause = format!("GROUP BY {}", group_columns.join(", "));

            let having_clause = if let QueryShape::GroupByHaving {
                having_predicate, ..
            } = shape
            {
                format!(" HAVING {having_predicate}")
            } else {
                String::new()
            };

            format!(
                "WITH data AS (SELECT {}) SELECT {} FROM data {group_by_clause}{having_clause}",
                cte_cols.join(", "),
                select_items.join(", ")
            )
        }
        QueryShape::Distinct => {
            // SELECT DISTINCT doesn't change types — just deduplicates
            let scalar_exprs: Vec<&TypedExpr> = exprs
                .iter()
                .filter(|e| !is_aggregate_expr(&e.sql) && !is_window_expr(&e.sql))
                .collect();
            let selected = if scalar_exprs.is_empty() {
                // fallback: use column refs
                vec![format!("{} AS expr_0", columns[0].name)]
            } else {
                scalar_exprs
                    .iter()
                    .map(|e| format!("{} AS {}", e.sql, e.alias))
                    .collect()
            };
            format!(
                "WITH data AS (SELECT {}) SELECT DISTINCT {} FROM data",
                cte_cols.join(", "),
                selected.join(", ")
            )
        }
        QueryShape::Scalar | QueryShape::Window => {
            let has_aggregate = exprs.iter().any(|e| is_aggregate_expr(&e.sql));
            let has_window = exprs.iter().any(|e| is_window_expr(&e.sql));

            let selected_exprs: Vec<&TypedExpr> = if has_aggregate && has_window {
                exprs
                    .iter()
                    .filter(|e| !is_aggregate_expr(&e.sql))
                    .collect()
            } else if has_aggregate {
                exprs.iter().filter(|e| is_aggregate_expr(&e.sql)).collect()
            } else {
                exprs.iter().collect()
            };

            let selected_exprs = if selected_exprs.is_empty() {
                exprs.iter().collect()
            } else {
                selected_exprs
            };

            let select_list: Vec<String> = selected_exprs
                .iter()
                .map(|e| format!("{} AS {}", e.sql, e.alias))
                .collect();

            format!(
                "WITH data AS (SELECT {}) SELECT {} FROM data",
                cte_cols.join(", "),
                select_list.join(", ")
            )
        }
    }
}

/// Wraps a generated query in a nested CTE, exercising the "inner query in CTE"
/// pattern. The inner query's output columns are preserved unchanged by the
/// outer `SELECT *`, so the expected output types are identical to the inner
/// query's. This stresses type propagation through a nested CTE boundary.
pub fn wrap_in_outer_cte(inner_sql: &str) -> String {
    format!("WITH wrapped AS ({inner_sql}) SELECT * FROM wrapped")
}

/// Generate a HAVING predicate for GROUP BY queries.
/// Returns a boolean-valued aggregate predicate string.
fn generate_having_predicate(columns: &[TypedSource], group_columns: &[String]) -> String {
    // Find a non-group column for aggregate predicates
    let agg_col = columns
        .iter()
        .find(|c| !group_columns.contains(&c.name))
        .map(|c| c.name.as_str());

    match agg_col {
        Some(col) => format!("COUNT({col}) > 0"),
        None => "COUNT(*) > 0".to_string(),
    }
}

/// Strategy that picks a query shape for the given column pool.
pub fn query_shape_strategy(
    columns: Vec<TypedSource>,
) -> impl Strategy<Value = (Vec<TypedSource>, QueryShape)> {
    let num_cols = columns.len();
    // Pick how many columns to use for GROUP BY (1..=min(3, num_cols))
    let max_group = num_cols.min(3);

    let cols_for_gb = columns.clone();
    let cols_for_gbh = columns.clone();
    let cols_for_gbw = columns.clone();

    // Weighted choice: Scalar 50%, GroupBy 15%, GroupByHaving 10%, GroupByWindow 10%, Window 5%, Distinct 10%
    prop_oneof![
        50 => Just((columns.clone(), QueryShape::Scalar)),
        5 => Just((columns.clone(), QueryShape::Window)),
        10 => Just((columns.clone(), QueryShape::Distinct)),
        15 => (1..=max_group).prop_map(move |n_group| {
            let group_cols: Vec<String> = cols_for_gb.iter().take(n_group).map(|c| c.name.clone()).collect();
            (cols_for_gb.clone(), QueryShape::GroupBy { group_columns: group_cols })
        }),
        10 => (1..=max_group).prop_map(move |n_group| {
            let group_cols: Vec<String> = cols_for_gbh.iter().take(n_group).map(|c| c.name.clone()).collect();
            let having = generate_having_predicate(&cols_for_gbh, &group_cols);
            (cols_for_gbh.clone(), QueryShape::GroupByHaving { group_columns: group_cols, having_predicate: having })
        }),
        10 => (1..=max_group).prop_map(move |n_group| {
            let group_cols: Vec<String> = cols_for_gbw.iter().take(n_group).map(|c| c.name.clone()).collect();
            (cols_for_gbw.clone(), QueryShape::GroupByWindow { group_columns: group_cols })
        }),
    ]
}

/// Strategy for generating complete test scenarios.
pub fn test_scenario_strategy(
) -> impl Strategy<Value = (Vec<TypedSource>, QueryShape, Vec<ExprKind>, Vec<usize>)> {
    column_pool_strategy()
        .prop_flat_map(query_shape_strategy)
        .prop_flat_map(|(cols, shape)| {
            let num_exprs = 1..=4usize;
            (
                Just(cols),
                Just(shape),
                prop::collection::vec(expr_kind_strategy(), num_exprs.clone()),
                prop::collection::vec(0..100usize, 1..=4),
            )
                .prop_filter("need at least one expr", |(_, _, kinds, _)| {
                    !kinds.is_empty()
                })
        })
}

// ---- Multi-model scenario for cross-model property tests ----

/// A generated two-model scenario for testing cross-model type propagation.
#[derive(Debug, Clone)]
pub struct MultiModelScenario {
    /// The upstream model SQL (valid SQL with CTE)
    pub upstream_sql: String,
    /// The upstream output columns with their types
    pub upstream_columns: Vec<TypedSource>,
    /// The downstream model SQL (uses smelt.models.upstream)
    pub downstream_sql: String,
    /// DuckDB-equivalent SQL (flattened CTEs, no smelt.ref)
    pub duckdb_sql: String,
    /// The downstream expressions with expected types
    pub downstream_exprs: Vec<TypedExpr>,
}

/// Generate a multi-model scenario: upstream model → downstream model with smelt.ref().
///
/// The upstream model can have either passthrough columns or computed expressions.
/// The downstream model applies scalar expressions (or optionally GROUP BY + aggregates)
/// over the upstream's output columns.
pub fn multi_model_scenario_strategy() -> impl Strategy<Value = MultiModelScenario> {
    column_pool_strategy()
        .prop_flat_map(|columns| {
            (
                Just(columns.clone()),
                // Upstream expression kinds (for non-passthrough upstream)
                prop::collection::vec(scalar_expr_kind_strategy(), 1..=3),
                prop::collection::vec(0..100usize, 1..=3),
                // Downstream expression kinds
                prop::collection::vec(scalar_expr_kind_strategy(), 1..=3),
                prop::collection::vec(0..100usize, 1..=3),
                // Whether to use expressions in upstream (50%) or passthrough
                proptest::bool::ANY,
                // Whether downstream uses GROUP BY (30%)
                prop::sample::select(vec![
                    false, false, false, false, false, false, true, true, true, false,
                ]),
            )
        })
        .prop_filter_map(
            "need valid scenario",
            |(
                columns,
                up_expr_kinds,
                up_func_indices,
                down_expr_kinds,
                down_func_indices,
                use_upstream_exprs,
                use_group_by,
            )| {
                let cte_cols: Vec<String> = columns
                    .iter()
                    .map(|c| format!("{} AS {}", c.cast_sql, c.name))
                    .collect();

                // Determine upstream output columns
                let (upstream_select_items, upstream_output_cols) = if use_upstream_exprs {
                    // Generate expressions for the upstream model
                    let mut up_exprs: Vec<TypedExpr> = Vec::new();
                    for (i, kind) in up_expr_kinds.iter().enumerate() {
                        let func_idx = up_func_indices.get(i).copied().unwrap_or(0);
                        if let Some(expr) = generate_expr(&columns, *kind, i, func_idx) {
                            up_exprs.push(expr);
                        }
                    }
                    if up_exprs.is_empty() {
                        return None;
                    }
                    // Rename upstream expr aliases to up_N to avoid collision
                    let items: Vec<String> = up_exprs
                        .iter()
                        .enumerate()
                        .map(|(i, e)| format!("{} AS up_{i}", e.sql))
                        .collect();
                    let out_cols: Vec<TypedSource> = up_exprs
                        .iter()
                        .enumerate()
                        .map(|(i, e)| TypedSource {
                            name: format!("up_{i}"),
                            data_type: e.expected_smelt_type.clone(),
                            cast_sql: String::new(), // not used for downstream
                        })
                        .collect();
                    (items, out_cols)
                } else {
                    // Passthrough
                    let items: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
                    (items, columns.clone())
                };

                let upstream_sql = format!(
                    "WITH data AS (SELECT {}) SELECT {} FROM data",
                    cte_cols.join(", "),
                    upstream_select_items.join(", ")
                );

                // Build downstream expressions
                if use_group_by && upstream_output_cols.len() >= 2 {
                    // GROUP BY: first column is group key, rest get aggregated
                    let group_col = &upstream_output_cols[0];
                    let agg_cols: Vec<&TypedSource> = upstream_output_cols.iter().skip(1).collect();

                    let mut select_items = vec![format!("{} AS grp_0", group_col.name)];
                    let mut down_exprs = vec![TypedExpr {
                        sql: group_col.name.clone(),
                        alias: "grp_0".to_string(),
                        expected_smelt_type: group_col.data_type.clone(),
                    }];

                    // Add COUNT(*) as a reliable aggregate
                    select_items.push("COUNT(*) AS agg_0".to_string());
                    down_exprs.push(TypedExpr {
                        sql: "COUNT(*)".to_string(),
                        alias: "agg_0".to_string(),
                        expected_smelt_type: DataType::BigInt,
                    });

                    // Add SUM/MIN/MAX on numeric columns
                    for (i, col) in agg_cols.iter().enumerate() {
                        if col.data_type.is_numeric() {
                            let agg_name = format!("agg_{}", i + 1);
                            select_items.push(format!("SUM({}) AS {agg_name}", col.name));
                            down_exprs.push(TypedExpr {
                                sql: format!("SUM({})", col.name),
                                alias: agg_name,
                                // SUM on integer types -> depends on input:
                                // DuckDB: SUM(INT) -> HUGEINT (mapped to Decimal), SUM(BIGINT) -> HUGEINT
                                // smelt: SUM(numeric) -> varies
                                // Use expected_smelt_type from the SUM inference
                                expected_smelt_type: DataType::unknown_dynamic(), // will be checked via DuckDB
                            });
                        }
                    }

                    let downstream_sql = format!(
                        "SELECT {} FROM smelt.models.upstream GROUP BY {}",
                        select_items.join(", "),
                        group_col.name
                    );

                    // DuckDB equivalent
                    let duckdb_cte = if use_upstream_exprs {
                        format!(
                            "WITH data AS (SELECT {}) , upstream AS (SELECT {} FROM data)",
                            cte_cols.join(", "),
                            upstream_select_items.join(", ")
                        )
                    } else {
                        format!("WITH upstream AS (SELECT {})", cte_cols.join(", "))
                    };
                    let duckdb_sql = format!(
                        "{} SELECT {} FROM upstream GROUP BY {}",
                        duckdb_cte,
                        select_items.join(", "),
                        group_col.name
                    );

                    Some(MultiModelScenario {
                        upstream_sql,
                        upstream_columns: upstream_output_cols,
                        downstream_sql,
                        duckdb_sql,
                        downstream_exprs: down_exprs,
                    })
                } else {
                    // Scalar downstream
                    let mut exprs: Vec<TypedExpr> = Vec::new();
                    for (i, kind) in down_expr_kinds.iter().enumerate() {
                        let func_idx = down_func_indices.get(i).copied().unwrap_or(0);
                        if let Some(expr) = generate_expr(&upstream_output_cols, *kind, i, func_idx)
                        {
                            exprs.push(expr);
                        }
                    }

                    if exprs.is_empty() {
                        return None;
                    }

                    let expr_items: Vec<String> = exprs
                        .iter()
                        .map(|e| format!("{} AS {}", e.sql, e.alias))
                        .collect();
                    let downstream_sql = format!(
                        "SELECT {} FROM smelt.models.upstream",
                        expr_items.join(", ")
                    );

                    let duckdb_cte = if use_upstream_exprs {
                        format!(
                            "WITH data AS (SELECT {}) , upstream AS (SELECT {} FROM data)",
                            cte_cols.join(", "),
                            upstream_select_items.join(", ")
                        )
                    } else {
                        format!("WITH upstream AS (SELECT {})", cte_cols.join(", "))
                    };
                    let duckdb_sql = format!(
                        "{} SELECT {} FROM upstream",
                        duckdb_cte,
                        expr_items.join(", ")
                    );

                    Some(MultiModelScenario {
                        upstream_sql,
                        upstream_columns: upstream_output_cols,
                        downstream_sql,
                        duckdb_sql,
                        downstream_exprs: exprs,
                    })
                }
            },
        )
}

// ---- Three-model chain scenario ----

/// A generated three-model chain for testing multi-hop type propagation.
#[derive(Debug, Clone)]
pub struct ThreeModelScenario {
    /// Model A: base model with CTE (no refs)
    pub model_a_sql: String,
    /// Model B: refs model A, applies expressions
    pub model_b_sql: String,
    /// Model C: refs model B, applies expressions
    pub model_c_sql: String,
    /// DuckDB-equivalent SQL (all three as CTEs)
    pub duckdb_sql: String,
}

/// Generate a three-model chain: A (base) → B (refs A) → C (refs B).
pub fn three_model_scenario_strategy() -> impl Strategy<Value = ThreeModelScenario> {
    column_pool_strategy()
        .prop_flat_map(|columns| {
            (
                Just(columns),
                // B's expression kinds over A's columns
                prop::collection::vec(scalar_expr_kind_strategy(), 1..=3),
                prop::collection::vec(0..100usize, 1..=3),
                // C's expression kinds over B's output
                prop::collection::vec(scalar_expr_kind_strategy(), 1..=3),
                prop::collection::vec(0..100usize, 1..=3),
            )
        })
        .prop_filter_map(
            "need valid three-model scenario",
            |(columns, b_expr_kinds, b_func_indices, c_expr_kinds, c_func_indices)| {
                // Model A: base CTE
                let cte_cols: Vec<String> = columns
                    .iter()
                    .map(|c| format!("{} AS {}", c.cast_sql, c.name))
                    .collect();
                let a_select: Vec<String> = columns.iter().map(|c| c.name.clone()).collect();
                let model_a_sql = format!(
                    "WITH data AS (SELECT {}) SELECT {} FROM data",
                    cte_cols.join(", "),
                    a_select.join(", ")
                );

                // Model B: refs A, applies expressions
                let mut b_exprs: Vec<TypedExpr> = Vec::new();
                for (i, kind) in b_expr_kinds.iter().enumerate() {
                    let func_idx = b_func_indices.get(i).copied().unwrap_or(0);
                    if let Some(mut expr) = generate_expr(&columns, *kind, i, func_idx) {
                        expr.alias = format!("b_{i}");
                        b_exprs.push(expr);
                    }
                }
                if b_exprs.is_empty() {
                    return None;
                }

                let b_items: Vec<String> = b_exprs
                    .iter()
                    .map(|e| format!("{} AS {}", e.sql, e.alias))
                    .collect();
                let model_b_sql = format!(
                    "SELECT {} FROM smelt.models.model_a",
                    b_items.join(", ")
                );

                // B's output columns
                let b_output: Vec<TypedSource> = b_exprs
                    .iter()
                    .map(|e| TypedSource {
                        name: e.alias.clone(),
                        data_type: e.expected_smelt_type.clone(),
                        cast_sql: String::new(),
                    })
                    .collect();

                // Model C: refs B, applies expressions
                let mut c_exprs: Vec<TypedExpr> = Vec::new();
                for (i, kind) in c_expr_kinds.iter().enumerate() {
                    let func_idx = c_func_indices.get(i).copied().unwrap_or(0);
                    if let Some(expr) = generate_expr(&b_output, *kind, i, func_idx) {
                        c_exprs.push(expr);
                    }
                }
                if c_exprs.is_empty() {
                    return None;
                }

                let c_items: Vec<String> = c_exprs
                    .iter()
                    .map(|e| format!("{} AS {}", e.sql, e.alias))
                    .collect();
                let model_c_sql = format!(
                    "SELECT {} FROM smelt.models.model_b",
                    c_items.join(", ")
                );

                // DuckDB equivalent: chain as CTEs
                let duckdb_sql = format!(
                    "WITH model_a AS (SELECT {}) , model_b AS (SELECT {} FROM model_a) SELECT {} FROM model_b",
                    cte_cols.join(", "),
                    b_items.join(", "),
                    c_items.join(", ")
                );

                Some(ThreeModelScenario {
                    model_a_sql,
                    model_b_sql,
                    model_c_sql,
                    duckdb_sql,
                })
            },
        )
}

// ---- JOIN scenario ----

/// A generated JOIN scenario: two upstream models joined in a downstream model.
#[derive(Debug, Clone)]
pub struct JoinScenario {
    /// Left upstream model SQL
    pub left_sql: String,
    /// Right upstream model SQL
    pub right_sql: String,
    /// Downstream model SQL (JOINs both upstreams via smelt.ref)
    pub join_sql: String,
    /// DuckDB-equivalent SQL
    pub duckdb_sql: String,
}

/// Generate a JOIN scenario: left_model INNER JOIN right_model ON shared key.
pub fn join_scenario_strategy() -> impl Strategy<Value = JoinScenario> {
    (column_pool_strategy(), column_pool_strategy())
        .prop_flat_map(|(left_cols, right_cols)| {
            (
                Just(left_cols),
                Just(right_cols),
                prop::collection::vec(scalar_expr_kind_strategy(), 1..=2),
                prop::collection::vec(0..100usize, 1..=2),
            )
        })
        .prop_filter_map(
            "need valid join scenario",
            |(left_cols, right_cols, expr_kinds, func_indices)| {
                if left_cols.is_empty() || right_cols.is_empty() {
                    return None;
                }

                // Build left model CTE
                let left_cte: Vec<String> = left_cols
                    .iter()
                    .map(|c| format!("{} AS {}", c.cast_sql, c.name))
                    .collect();
                // Add a join key: CAST(1 AS INTEGER) AS join_key
                let left_sql = format!(
                    "WITH data AS (SELECT {}, CAST(1 AS INTEGER) AS join_key) SELECT {}, join_key FROM data",
                    left_cte.join(", "),
                    left_cols.iter().map(|c| c.name.as_str()).collect::<Vec<_>>().join(", ")
                );

                // Build right model CTE — prefix column names with r_ to avoid collision
                let right_cte: Vec<String> = right_cols
                    .iter()
                    .map(|c| format!("{} AS r_{}", c.cast_sql, c.name))
                    .collect();
                let right_sql = format!(
                    "WITH data AS (SELECT {}, CAST(1 AS INTEGER) AS join_key) SELECT {}, join_key FROM data",
                    right_cte.join(", "),
                    right_cols.iter().map(|c| format!("r_{}", c.name)).collect::<Vec<_>>().join(", ")
                );

                // Combined columns available after JOIN (qualified by alias)
                let mut combined_cols: Vec<TypedSource> = Vec::new();
                for c in &left_cols {
                    combined_cols.push(TypedSource {
                        name: format!("l.{}", c.name),
                        data_type: c.data_type.clone(),
                        cast_sql: String::new(),
                    });
                }
                for c in &right_cols {
                    combined_cols.push(TypedSource {
                        name: format!("r.r_{}", c.name),
                        data_type: c.data_type.clone(),
                        cast_sql: String::new(),
                    });
                }

                // Generate expressions over combined columns
                let mut exprs: Vec<TypedExpr> = Vec::new();
                for (i, _kind) in expr_kinds.iter().enumerate() {
                    let func_idx = func_indices.get(i).copied().unwrap_or(0);
                    // Only use ColumnRef for join tests to keep things simple
                    if let Some(expr) = generate_expr(&combined_cols, ExprKind::ColumnRef, i, func_idx) {
                        exprs.push(expr);
                    }
                }
                if exprs.is_empty() {
                    return None;
                }

                let expr_items: Vec<String> = exprs
                    .iter()
                    .map(|e| format!("{} AS {}", e.sql, e.alias))
                    .collect();

                let join_sql = format!(
                    "SELECT {} FROM smelt.models.left_model l INNER JOIN smelt.models.right_model r ON l.join_key = r.join_key",
                    expr_items.join(", ")
                );

                let duckdb_sql = format!(
                    "WITH left_model AS (SELECT {}, CAST(1 AS INTEGER) AS join_key) , right_model AS (SELECT {}, CAST(1 AS INTEGER) AS join_key) SELECT {} FROM left_model l INNER JOIN right_model r ON l.join_key = r.join_key",
                    left_cte.iter().zip(left_cols.iter()).map(|(_cte, c)| format!("{} AS {}", c.cast_sql, c.name)).collect::<Vec<_>>().join(", "),
                    right_cols.iter().map(|c| format!("{} AS r_{}", c.cast_sql, c.name)).collect::<Vec<_>>().join(", "),
                    expr_items.join(", ")
                );

                Some(JoinScenario {
                    left_sql,
                    right_sql,
                    join_sql,
                    duckdb_sql,
                })
            },
        )
}

/// Expression kinds that work in scalar (non-GROUP BY) context.
fn scalar_expr_kind_strategy() -> impl Strategy<Value = ExprKind> {
    prop_oneof![
        3 => Just(ExprKind::ColumnRef),
        3 => Just(ExprKind::Function),
        2 => Just(ExprKind::BinaryOp),
        1 => Just(ExprKind::CaseExpr),
        1 => Just(ExprKind::Cast),
        1 => Just(ExprKind::IsNull),
        1 => Just(ExprKind::Comparison),
        1 => Just(ExprKind::UnaryNot),
        1 => Just(ExprKind::UnaryMinus),
    ]
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
        let sql = assemble_cte_query(&cols, &exprs, &QueryShape::Scalar);
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
        let sql = assemble_cte_query(&cols, &exprs, &QueryShape::Scalar);
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

    #[test]
    fn assemble_group_by_query() {
        let cols = vec![
            TypedSource {
                name: "str_col_0".into(),
                data_type: DataType::Varchar { max_length: None },
                cast_sql: "CAST('hello' AS STRING)".into(),
            },
            TypedSource {
                name: "int_col_1".into(),
                data_type: DataType::Integer,
                cast_sql: "CAST(42 AS INTEGER)".into(),
            },
        ];
        let exprs = vec![TypedExpr {
            sql: "COUNT(int_col_1)".into(),
            alias: "expr_0".into(),
            expected_smelt_type: DataType::BigInt,
        }];
        let shape = QueryShape::GroupBy {
            group_columns: vec!["str_col_0".into()],
        };
        let sql = assemble_cte_query(&cols, &exprs, &shape);
        assert!(sql.contains("GROUP BY str_col_0"));
        assert!(sql.contains("str_col_0 AS grp_0"));
        assert!(sql.contains("COUNT(int_col_1) AS expr_0"));
        assert!(!sql.contains("HAVING"));
    }

    #[test]
    fn assemble_group_by_having_query() {
        let cols = vec![
            TypedSource {
                name: "str_col_0".into(),
                data_type: DataType::Varchar { max_length: None },
                cast_sql: "CAST('hello' AS STRING)".into(),
            },
            TypedSource {
                name: "int_col_1".into(),
                data_type: DataType::Integer,
                cast_sql: "CAST(42 AS INTEGER)".into(),
            },
        ];
        let exprs = vec![TypedExpr {
            sql: "SUM(int_col_1)".into(),
            alias: "expr_0".into(),
            expected_smelt_type: DataType::Decimal {
                precision: 38,
                scale: 10,
            },
        }];
        let shape = QueryShape::GroupByHaving {
            group_columns: vec!["str_col_0".into()],
            having_predicate: "COUNT(int_col_1) > 0".into(),
        };
        let sql = assemble_cte_query(&cols, &exprs, &shape);
        assert!(sql.contains("GROUP BY str_col_0"));
        assert!(sql.contains("HAVING COUNT(int_col_1) > 0"));
    }

    #[test]
    fn assemble_group_by_all_columns_uses_count_star() {
        let cols = vec![TypedSource {
            name: "int_col_0".into(),
            data_type: DataType::Integer,
            cast_sql: "CAST(42 AS INTEGER)".into(),
        }];
        // No aggregate expressions provided
        let exprs = vec![TypedExpr {
            sql: "int_col_0".into(),
            alias: "expr_0".into(),
            expected_smelt_type: DataType::Integer,
        }];
        let shape = QueryShape::GroupBy {
            group_columns: vec!["int_col_0".into()],
        };
        let sql = assemble_cte_query(&cols, &exprs, &shape);
        assert!(sql.contains("GROUP BY int_col_0"));
        assert!(sql.contains("COUNT(*) AS expr_0"));
    }
}
