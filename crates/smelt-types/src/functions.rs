/// Categories of SQL functions
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FunctionCategory {
    Aggregate,
    WindowRanking,
    WindowDistribution,
    WindowNavigation,
    NullHandling,
    DateTime,
    String,
    Math,
    Trigonometric,
    Comparison,
    Json,
    Boolean,
    Array,
    TypeConversion,
    Constant,
}

/// Known SQL function names used across smelt crates.
///
/// Every function that appears in type inference, optimizer analysis,
/// diagnostics, or test generators should have a variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SqlFunction {
    // Aggregate functions
    Count,
    Sum,
    Avg,
    Min,
    Max,
    Stddev,
    Variance,
    StddevPop,
    StddevSamp,
    VarPop,
    VarSamp,
    ArrayAgg,
    StringAgg,
    GroupConcat,
    Listagg,
    Median,
    Mode,
    PercentileCont,
    PercentileDisc,
    ApproxCountDistinct,
    AnyValue,
    ArgMax,
    First,
    Last,
    BoolAnd,
    BoolOr,
    BitAnd,
    BitOr,
    BitXor,
    Corr,
    CovarPop,
    CovarSamp,
    RegrSlope,
    Every,

    // Window ranking functions
    RowNumber,
    Rank,
    DenseRank,
    Ntile,

    // Window distribution functions
    CumeDist,
    PercentRank,

    // Window navigation functions
    Lag,
    Lead,
    FirstValue,
    LastValue,
    NthValue,

    // Null handling
    Coalesce,
    Nullif,
    Ifnull,

    // Date/time functions
    Now,
    CurrentTimestamp,
    CurrentDate,
    Date,
    DateTrunc,
    Extract,
    DatePart,
    Year,
    Month,
    Day,
    DayOfWeek,
    Quarter,
    MakeDate,
    MakeTime,
    MakeTimestamp,
    MakeTimestamptz,
    Age,
    ToSeconds,

    // String functions
    Concat,
    Upper,
    Lower,
    Md5,
    Trim,
    Ltrim,
    Rtrim,
    Substring,
    Substr,
    Length,
    CharLength,
    CharacterLength,
    ToChar,
    Replace,
    Translate,
    Reverse,
    Repeat,
    Lpad,
    Rpad,
    Initcap,
    QuoteIdent,
    QuoteLiteral,
    Left,
    Right,
    Position,
    Strpos,
    SplitPart,

    // Math functions
    Abs,
    Sign,
    Round,
    Trunc,
    Truncate,
    Ceil,
    Ceiling,
    Floor,
    Power,
    Pow,
    Sqrt,
    Exp,
    Ln,
    Log,
    Log10,
    Log2,
    Mod,

    // Trigonometric functions
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Sinh,
    Cosh,
    Tanh,

    // Constants / zero-arg functions
    Pi,
    Random,

    // Comparison functions
    Greatest,
    Least,

    // JSON functions (canonical names — accept dialect aliases via from_name)
    /// json_object / json_build_object — construct JSON object from key-value pairs
    JsonObject,
    /// json_array / json_build_array — construct JSON array from values
    JsonArray,
    /// to_json / to_jsonb / row_to_json — convert value to JSON
    ToJson,
    /// json_extract / json_extract_path — extract JSON subtree (returns JSON)
    JsonExtract,
    /// json_extract_string / json_extract_text / json_extract_path_text / get_json_object — extract as text
    JsonExtractText,
    /// json_array_length — number of elements in JSON array
    JsonArrayLength,
    /// json_object_keys / json_keys — keys of JSON object
    JsonObjectKeys,
    /// json_contains — JSON containment check
    JsonContains,
    // Boolean aggregate (also listed under aggregate)
    // (BoolAnd, BoolOr, Every already above)
}

/// All variants in definition order. Used by `SqlFunction::all()`.
const ALL_FUNCTIONS: &[SqlFunction] = &[
    SqlFunction::Count,
    SqlFunction::Sum,
    SqlFunction::Avg,
    SqlFunction::Min,
    SqlFunction::Max,
    SqlFunction::Stddev,
    SqlFunction::Variance,
    SqlFunction::StddevPop,
    SqlFunction::StddevSamp,
    SqlFunction::VarPop,
    SqlFunction::VarSamp,
    SqlFunction::ArrayAgg,
    SqlFunction::StringAgg,
    SqlFunction::GroupConcat,
    SqlFunction::Listagg,
    SqlFunction::Median,
    SqlFunction::Mode,
    SqlFunction::PercentileCont,
    SqlFunction::PercentileDisc,
    SqlFunction::ApproxCountDistinct,
    SqlFunction::AnyValue,
    SqlFunction::ArgMax,
    SqlFunction::First,
    SqlFunction::Last,
    SqlFunction::BoolAnd,
    SqlFunction::BoolOr,
    SqlFunction::BitAnd,
    SqlFunction::BitOr,
    SqlFunction::BitXor,
    SqlFunction::Corr,
    SqlFunction::CovarPop,
    SqlFunction::CovarSamp,
    SqlFunction::RegrSlope,
    SqlFunction::Every,
    SqlFunction::RowNumber,
    SqlFunction::Rank,
    SqlFunction::DenseRank,
    SqlFunction::Ntile,
    SqlFunction::CumeDist,
    SqlFunction::PercentRank,
    SqlFunction::Lag,
    SqlFunction::Lead,
    SqlFunction::FirstValue,
    SqlFunction::LastValue,
    SqlFunction::NthValue,
    SqlFunction::Coalesce,
    SqlFunction::Nullif,
    SqlFunction::Ifnull,
    SqlFunction::Now,
    SqlFunction::CurrentTimestamp,
    SqlFunction::CurrentDate,
    SqlFunction::Date,
    SqlFunction::DateTrunc,
    SqlFunction::Extract,
    SqlFunction::DatePart,
    SqlFunction::Year,
    SqlFunction::Month,
    SqlFunction::Day,
    SqlFunction::DayOfWeek,
    SqlFunction::Quarter,
    SqlFunction::MakeDate,
    SqlFunction::MakeTime,
    SqlFunction::MakeTimestamp,
    SqlFunction::MakeTimestamptz,
    SqlFunction::Age,
    SqlFunction::ToSeconds,
    SqlFunction::Concat,
    SqlFunction::Upper,
    SqlFunction::Lower,
    SqlFunction::Md5,
    SqlFunction::Trim,
    SqlFunction::Ltrim,
    SqlFunction::Rtrim,
    SqlFunction::Substring,
    SqlFunction::Substr,
    SqlFunction::Length,
    SqlFunction::CharLength,
    SqlFunction::CharacterLength,
    SqlFunction::ToChar,
    SqlFunction::Replace,
    SqlFunction::Translate,
    SqlFunction::Reverse,
    SqlFunction::Repeat,
    SqlFunction::Lpad,
    SqlFunction::Rpad,
    SqlFunction::Initcap,
    SqlFunction::QuoteIdent,
    SqlFunction::QuoteLiteral,
    SqlFunction::Left,
    SqlFunction::Right,
    SqlFunction::Position,
    SqlFunction::Strpos,
    SqlFunction::SplitPart,
    SqlFunction::Abs,
    SqlFunction::Sign,
    SqlFunction::Round,
    SqlFunction::Trunc,
    SqlFunction::Truncate,
    SqlFunction::Ceil,
    SqlFunction::Ceiling,
    SqlFunction::Floor,
    SqlFunction::Power,
    SqlFunction::Pow,
    SqlFunction::Sqrt,
    SqlFunction::Exp,
    SqlFunction::Ln,
    SqlFunction::Log,
    SqlFunction::Log10,
    SqlFunction::Log2,
    SqlFunction::Mod,
    SqlFunction::Sin,
    SqlFunction::Cos,
    SqlFunction::Tan,
    SqlFunction::Asin,
    SqlFunction::Acos,
    SqlFunction::Atan,
    SqlFunction::Atan2,
    SqlFunction::Sinh,
    SqlFunction::Cosh,
    SqlFunction::Tanh,
    SqlFunction::Pi,
    SqlFunction::Random,
    SqlFunction::Greatest,
    SqlFunction::Least,
    SqlFunction::JsonObject,
    SqlFunction::JsonArray,
    SqlFunction::ToJson,
    SqlFunction::JsonExtract,
    SqlFunction::JsonExtractText,
    SqlFunction::JsonArrayLength,
    SqlFunction::JsonObjectKeys,
    SqlFunction::JsonContains,
];

impl SqlFunction {
    /// Look up a function by name (case-insensitive).
    ///
    /// Accepts both canonical smelt names and dialect-specific aliases
    /// (e.g., `JSON_BUILD_OBJECT` → `JsonObject`, `GET_JSON_OBJECT` → `JsonExtractText`).
    ///
    /// Alias resolution is owned by [`crate::signatures::BuiltinRegistry`]
    /// (architecture.md §Constraints #14, "Function-registry single
    /// ownership"): a dialect spelling is recognized, classified, and typed
    /// from exactly one row — the registry's canonical entry plus its
    /// `aliases` table — never a second alias-only mapping here.
    pub fn from_name(name: &str) -> Option<Self> {
        let canonical = crate::signatures::BuiltinRegistry::canonical_name(name)?;
        ALL_FUNCTIONS
            .iter()
            .find(|f| f.name() == canonical)
            .copied()
    }

    /// Canonical uppercase SQL name.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Count => "COUNT",
            Self::Sum => "SUM",
            Self::Avg => "AVG",
            Self::Min => "MIN",
            Self::Max => "MAX",
            Self::Stddev => "STDDEV",
            Self::Variance => "VARIANCE",
            Self::StddevPop => "STDDEV_POP",
            Self::StddevSamp => "STDDEV_SAMP",
            Self::VarPop => "VAR_POP",
            Self::VarSamp => "VAR_SAMP",
            Self::ArrayAgg => "ARRAY_AGG",
            Self::StringAgg => "STRING_AGG",
            Self::GroupConcat => "GROUP_CONCAT",
            Self::Listagg => "LISTAGG",
            Self::Median => "MEDIAN",
            Self::Mode => "MODE",
            Self::PercentileCont => "PERCENTILE_CONT",
            Self::PercentileDisc => "PERCENTILE_DISC",
            Self::ApproxCountDistinct => "APPROX_COUNT_DISTINCT",
            Self::AnyValue => "ANY_VALUE",
            Self::ArgMax => "ARG_MAX",
            Self::First => "FIRST",
            Self::Last => "LAST",
            Self::BoolAnd => "BOOL_AND",
            Self::BoolOr => "BOOL_OR",
            Self::BitAnd => "BIT_AND",
            Self::BitOr => "BIT_OR",
            Self::BitXor => "BIT_XOR",
            Self::Corr => "CORR",
            Self::CovarPop => "COVAR_POP",
            Self::CovarSamp => "COVAR_SAMP",
            Self::RegrSlope => "REGR_SLOPE",
            Self::Every => "EVERY",
            Self::RowNumber => "ROW_NUMBER",
            Self::Rank => "RANK",
            Self::DenseRank => "DENSE_RANK",
            Self::Ntile => "NTILE",
            Self::CumeDist => "CUME_DIST",
            Self::PercentRank => "PERCENT_RANK",
            Self::Lag => "LAG",
            Self::Lead => "LEAD",
            Self::FirstValue => "FIRST_VALUE",
            Self::LastValue => "LAST_VALUE",
            Self::NthValue => "NTH_VALUE",
            Self::Coalesce => "COALESCE",
            Self::Nullif => "NULLIF",
            Self::Ifnull => "IFNULL",
            Self::Now => "NOW",
            Self::CurrentTimestamp => "CURRENT_TIMESTAMP",
            Self::CurrentDate => "CURRENT_DATE",
            Self::Date => "DATE",
            Self::DateTrunc => "DATE_TRUNC",
            Self::Extract => "EXTRACT",
            Self::DatePart => "DATE_PART",
            Self::Year => "YEAR",
            Self::Month => "MONTH",
            Self::Day => "DAY",
            Self::DayOfWeek => "DAYOFWEEK",
            Self::Quarter => "QUARTER",
            Self::MakeDate => "MAKE_DATE",
            Self::MakeTime => "MAKE_TIME",
            Self::MakeTimestamp => "MAKE_TIMESTAMP",
            Self::MakeTimestamptz => "MAKE_TIMESTAMPTZ",
            Self::Age => "AGE",
            Self::ToSeconds => "TO_SECONDS",
            Self::Concat => "CONCAT",
            Self::Upper => "UPPER",
            Self::Lower => "LOWER",
            Self::Md5 => "MD5",
            Self::Trim => "TRIM",
            Self::Ltrim => "LTRIM",
            Self::Rtrim => "RTRIM",
            Self::Substring => "SUBSTRING",
            Self::Substr => "SUBSTR",
            Self::Length => "LENGTH",
            Self::CharLength => "CHAR_LENGTH",
            Self::CharacterLength => "CHARACTER_LENGTH",
            Self::ToChar => "TO_CHAR",
            Self::Replace => "REPLACE",
            Self::Translate => "TRANSLATE",
            Self::Reverse => "REVERSE",
            Self::Repeat => "REPEAT",
            Self::Lpad => "LPAD",
            Self::Rpad => "RPAD",
            Self::Initcap => "INITCAP",
            Self::QuoteIdent => "QUOTE_IDENT",
            Self::QuoteLiteral => "QUOTE_LITERAL",
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
            Self::Position => "POSITION",
            Self::Strpos => "STRPOS",
            Self::SplitPart => "SPLIT_PART",
            Self::Abs => "ABS",
            Self::Sign => "SIGN",
            Self::Round => "ROUND",
            Self::Trunc => "TRUNC",
            Self::Truncate => "TRUNCATE",
            Self::Ceil => "CEIL",
            Self::Ceiling => "CEILING",
            Self::Floor => "FLOOR",
            Self::Power => "POWER",
            Self::Pow => "POW",
            Self::Sqrt => "SQRT",
            Self::Exp => "EXP",
            Self::Ln => "LN",
            Self::Log => "LOG",
            Self::Log10 => "LOG10",
            Self::Log2 => "LOG2",
            Self::Mod => "MOD",
            Self::Sin => "SIN",
            Self::Cos => "COS",
            Self::Tan => "TAN",
            Self::Asin => "ASIN",
            Self::Acos => "ACOS",
            Self::Atan => "ATAN",
            Self::Atan2 => "ATAN2",
            Self::Sinh => "SINH",
            Self::Cosh => "COSH",
            Self::Tanh => "TANH",
            Self::Pi => "PI",
            Self::Random => "RANDOM",
            Self::Greatest => "GREATEST",
            Self::Least => "LEAST",
            Self::JsonObject => "JSON_OBJECT",
            Self::JsonArray => "JSON_ARRAY",
            Self::ToJson => "TO_JSON",
            Self::JsonExtract => "JSON_EXTRACT",
            Self::JsonExtractText => "JSON_EXTRACT_TEXT",
            Self::JsonArrayLength => "JSON_ARRAY_LENGTH",
            Self::JsonObjectKeys => "JSON_OBJECT_KEYS",
            Self::JsonContains => "JSON_CONTAINS",
        }
    }

    /// Function category.
    pub fn category(&self) -> FunctionCategory {
        match self {
            Self::Count
            | Self::Sum
            | Self::Avg
            | Self::Min
            | Self::Max
            | Self::Stddev
            | Self::Variance
            | Self::StddevPop
            | Self::StddevSamp
            | Self::VarPop
            | Self::VarSamp
            | Self::ArrayAgg
            | Self::StringAgg
            | Self::GroupConcat
            | Self::Listagg
            | Self::Median
            | Self::Mode
            | Self::PercentileCont
            | Self::PercentileDisc
            | Self::ApproxCountDistinct
            | Self::AnyValue
            | Self::ArgMax
            | Self::First
            | Self::Last
            | Self::BoolAnd
            | Self::BoolOr
            | Self::BitAnd
            | Self::BitOr
            | Self::BitXor
            | Self::Corr
            | Self::CovarPop
            | Self::CovarSamp
            | Self::RegrSlope
            | Self::Every => FunctionCategory::Aggregate,

            Self::RowNumber | Self::Rank | Self::DenseRank | Self::Ntile => {
                FunctionCategory::WindowRanking
            }

            Self::CumeDist | Self::PercentRank => FunctionCategory::WindowDistribution,

            Self::Lag | Self::Lead | Self::FirstValue | Self::LastValue | Self::NthValue => {
                FunctionCategory::WindowNavigation
            }

            Self::Coalesce | Self::Nullif | Self::Ifnull => FunctionCategory::NullHandling,

            Self::Now
            | Self::CurrentTimestamp
            | Self::CurrentDate
            | Self::Date
            | Self::DateTrunc
            | Self::Extract
            | Self::DatePart
            | Self::Year
            | Self::Month
            | Self::Day
            | Self::DayOfWeek
            | Self::Quarter
            | Self::MakeDate
            | Self::MakeTime
            | Self::MakeTimestamp
            | Self::MakeTimestamptz
            | Self::Age
            | Self::ToSeconds => FunctionCategory::DateTime,

            Self::Concat
            | Self::Upper
            | Self::Lower
            | Self::Md5
            | Self::Trim
            | Self::Ltrim
            | Self::Rtrim
            | Self::Substring
            | Self::Substr
            | Self::Length
            | Self::CharLength
            | Self::CharacterLength
            | Self::ToChar
            | Self::Replace
            | Self::Translate
            | Self::Reverse
            | Self::Repeat
            | Self::Lpad
            | Self::Rpad
            | Self::Initcap
            | Self::QuoteIdent
            | Self::QuoteLiteral
            | Self::Left
            | Self::Right
            | Self::Position
            | Self::Strpos
            | Self::SplitPart => FunctionCategory::String,

            Self::Abs
            | Self::Sign
            | Self::Round
            | Self::Trunc
            | Self::Truncate
            | Self::Ceil
            | Self::Ceiling
            | Self::Floor
            | Self::Power
            | Self::Pow
            | Self::Sqrt
            | Self::Exp
            | Self::Ln
            | Self::Log
            | Self::Log10
            | Self::Log2
            | Self::Mod => FunctionCategory::Math,

            Self::Sin
            | Self::Cos
            | Self::Tan
            | Self::Asin
            | Self::Acos
            | Self::Atan
            | Self::Atan2
            | Self::Sinh
            | Self::Cosh
            | Self::Tanh => FunctionCategory::Trigonometric,

            Self::Pi | Self::Random => FunctionCategory::Constant,

            Self::Greatest | Self::Least => FunctionCategory::Comparison,

            Self::JsonObject
            | Self::JsonArray
            | Self::ToJson
            | Self::JsonExtract
            | Self::JsonExtractText
            | Self::JsonArrayLength
            | Self::JsonObjectKeys
            | Self::JsonContains => FunctionCategory::Json,
        }
    }

    /// Whether this function is an aggregate function.
    pub fn is_aggregate(&self) -> bool {
        self.category() == FunctionCategory::Aggregate
    }

    /// Whether this function is a window function (ranking, distribution, or navigation).
    pub fn is_window(&self) -> bool {
        matches!(
            self.category(),
            FunctionCategory::WindowRanking
                | FunctionCategory::WindowDistribution
                | FunctionCategory::WindowNavigation
        )
    }

    /// Iterator over all known SQL functions.
    pub fn all() -> impl Iterator<Item = SqlFunction> {
        ALL_FUNCTIONS.iter().copied()
    }
}

impl std::fmt::Display for SqlFunction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_all_variants() {
        for f in SqlFunction::all() {
            assert_eq!(
                SqlFunction::from_name(f.name()),
                Some(f),
                "round-trip failed for {:?} (name={})",
                f,
                f.name()
            );
        }
    }

    #[test]
    fn from_name_case_insensitive() {
        assert_eq!(SqlFunction::from_name("count"), Some(SqlFunction::Count));
        assert_eq!(SqlFunction::from_name("Count"), Some(SqlFunction::Count));
        assert_eq!(SqlFunction::from_name("COUNT"), Some(SqlFunction::Count));
    }

    #[test]
    fn from_name_unknown_returns_none() {
        assert_eq!(SqlFunction::from_name("not_a_function"), None);
    }

    #[test]
    fn aggregate_classification() {
        assert!(SqlFunction::Count.is_aggregate());
        assert!(SqlFunction::Sum.is_aggregate());
        assert!(SqlFunction::Stddev.is_aggregate());
        assert!(!SqlFunction::RowNumber.is_aggregate());
        assert!(!SqlFunction::Upper.is_aggregate());
    }

    #[test]
    fn window_classification() {
        assert!(SqlFunction::RowNumber.is_window());
        assert!(SqlFunction::Lag.is_window());
        assert!(SqlFunction::CumeDist.is_window());
        assert!(!SqlFunction::Count.is_window());
    }

    #[test]
    fn all_variants_have_category() {
        // Ensures the category match is exhaustive at compile time
        // (it already is, but this test exercises the runtime path)
        for f in SqlFunction::all() {
            let _ = f.category();
        }
    }

    #[test]
    fn json_dialect_aliases() {
        // PostgreSQL names
        assert_eq!(
            SqlFunction::from_name("json_build_object"),
            Some(SqlFunction::JsonObject)
        );
        assert_eq!(
            SqlFunction::from_name("json_build_array"),
            Some(SqlFunction::JsonArray)
        );
        assert_eq!(
            SqlFunction::from_name("to_jsonb"),
            Some(SqlFunction::ToJson)
        );
        assert_eq!(
            SqlFunction::from_name("row_to_json"),
            Some(SqlFunction::ToJson)
        );
        assert_eq!(
            SqlFunction::from_name("json_extract_path_text"),
            Some(SqlFunction::JsonExtractText)
        );

        // DuckDB names
        assert_eq!(
            SqlFunction::from_name("json_extract_string"),
            Some(SqlFunction::JsonExtractText)
        );
        assert_eq!(
            SqlFunction::from_name("json_keys"),
            Some(SqlFunction::JsonObjectKeys)
        );

        // Spark names
        assert_eq!(
            SqlFunction::from_name("get_json_object"),
            Some(SqlFunction::JsonExtractText)
        );

        // Canonical names
        assert_eq!(
            SqlFunction::from_name("json_object"),
            Some(SqlFunction::JsonObject)
        );
        assert_eq!(
            SqlFunction::from_name("json_extract"),
            Some(SqlFunction::JsonExtract)
        );
        assert_eq!(
            SqlFunction::from_name("json_contains"),
            Some(SqlFunction::JsonContains)
        );
    }
}
