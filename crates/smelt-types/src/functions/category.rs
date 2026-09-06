use super::{FunctionCategory, SqlFunction};

impl SqlFunction {
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
            | Self::ArgMin
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
            | Self::ToSeconds
            | Self::DateAdd
            | Self::DateSub => FunctionCategory::DateTime,

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
}
