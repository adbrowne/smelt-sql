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
    ArgMin,
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
    DateAdd,
    DateSub,

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
    SqlFunction::ArgMin,
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
    SqlFunction::DateAdd,
    SqlFunction::DateSub,
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

mod category;
mod name;
#[cfg(test)]
mod tests;

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
