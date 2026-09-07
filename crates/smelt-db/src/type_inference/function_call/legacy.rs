//! The hand-written `match` over [`SqlFunction`] for every built-in not yet
//! migrated to the registry (see [`super::registry::REGISTRY_MIGRATED`]).

use smelt_parser::ast::FunctionCall;
use smelt_types::{DataType, SqlFunction, TypedColumn};

use super::registry::try_registry_inference;
use crate::type_inference::type_context::TypeContext;
use crate::type_inference::{infer_expression_type, promote_types};

/// Infer the type of a function call (aggregates, etc.)
pub fn infer_function_type(func: &FunctionCall, ctx: &TypeContext) -> Option<TypedColumn> {
    let name = func.name()?.to_uppercase();
    let sql_func = SqlFunction::from_name(&name)?;

    // Phase 9: registry-first lookup for the allowlisted subset. When the
    // registry returns a concrete result we use it; on miss/fall-through we
    // continue into the legacy match below.
    if let Some(Some(tc)) = try_registry_inference(&name, func, ctx) {
        return Some(tc);
    }

    /// Helper: return the type of the first argument, or `fallback` if inference fails.
    /// For COALESCE and similar, this intentionally only checks the first arg —
    /// using a later arg's type would risk being incorrect if earlier args are Unknown.
    fn first_arg_type_or(
        func: &FunctionCall,
        ctx: &TypeContext,
        fallback: DataType,
        nullable: bool,
    ) -> Option<TypedColumn> {
        if let Some(arg) = func.arguments().first() {
            if let Some(arg_type) = infer_expression_type(arg, ctx) {
                return Some(TypedColumn {
                    data_type: arg_type.data_type,
                    nullable,
                });
            }
        }
        Some(TypedColumn {
            data_type: fallback,
            nullable,
        })
    }

    /// Fold every argument's type through `promote_types` (the same widening
    /// `CASE`/`UNION` use) in argument order. `promote_types` already treats
    /// `Unknown`/`Null` as dominated by any known type, so an Unknown-typed
    /// leading argument doesn't poison the fold — used by COALESCE, IFNULL,
    /// GREATEST, LEAST, and MOD so mixed numeric argument types widen to their
    /// common type instead of returning the first argument's type verbatim.
    fn promote_arg_types(func: &FunctionCall, ctx: &TypeContext) -> Option<DataType> {
        let mut result: Option<TypedColumn> = None;
        for arg in func.arguments() {
            if let Some(arg_type) = infer_expression_type(&arg, ctx) {
                result = Some(match result {
                    None => arg_type,
                    Some(acc) => promote_types(&acc, &arg_type),
                });
            }
        }
        result.map(|tc| tc.data_type)
    }

    match sql_func {
        SqlFunction::Count => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: false,
        }),

        SqlFunction::Sum => {
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    // DuckDB SUM widening rules:
                    //   SUM(SMALLINT|INTEGER|BIGINT)   -> BIGINT (HUGEINT in DuckDB,
                    //                                     but smelt models that as BIGINT)
                    //   SUM(DOUBLE|FLOAT)              -> DOUBLE
                    //   SUM(DECIMAL(p, s))             -> DECIMAL(38, s)
                    //
                    // The Decimal precision widen-to-38 is critical: real
                    // pipelines accumulate ~1e6 rows of DECIMAL(10,2) values
                    // which overflow precision 10 quickly. Keeping the input
                    // precision silently corrupts results.
                    let result_type = match &arg_type.data_type {
                        DataType::SmallInt | DataType::Integer => DataType::BigInt,
                        DataType::BigInt => DataType::BigInt,
                        DataType::Float | DataType::Double => DataType::Double,
                        DataType::Decimal { scale, .. } => DataType::Decimal {
                            precision: 38,
                            scale: *scale,
                        },
                        // Unknown / mixed: defer to BIGINT (the historical
                        // fallback) — but the caller is expected to give us
                        // a populated TypeContext so this path is rare.
                        _ => DataType::BigInt,
                    };
                    return Some(TypedColumn {
                        data_type: result_type,
                        nullable: true,
                    });
                }
            }
            Some(TypedColumn {
                data_type: DataType::BigInt,
                nullable: true,
            })
        }

        SqlFunction::Avg => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),

        SqlFunction::Min | SqlFunction::Max => {
            first_arg_type_or(func, ctx, DataType::unknown_dynamic(), true)
        }

        SqlFunction::Coalesce => {
            // Fold all argument types via promote_types (the same widening used
            // by CASE/UNION) rather than taking the first concrete type verbatim
            // — DuckDB and Spark both widen COALESCE across mixed argument types
            // (e.g. COALESCE(SMALLINT, INTEGER) -> INTEGER), and taking only the
            // first concrete type disagreed with DuckDB itself, not just Spark.
            // COALESCE is non-nullable when at least one argument is non-nullable
            // or is a non-null literal, because the result will always have a value.
            let mut has_non_nullable_arg = false;
            for arg in func.arguments() {
                if let Some(arg_type) = infer_expression_type(&arg, ctx) {
                    if !arg_type.nullable {
                        has_non_nullable_arg = true;
                    }
                }
            }
            let data_type = promote_arg_types(func, ctx)
                .unwrap_or(DataType::Unknown(smelt_types::UnknownReason::Dynamic));
            Some(TypedColumn {
                data_type,
                nullable: !has_non_nullable_arg,
            })
        }

        SqlFunction::Nullif => first_arg_type_or(func, ctx, DataType::unknown_dynamic(), true),

        SqlFunction::Ifnull => {
            // IFNULL(a, b) is equivalent to COALESCE(a, b) — same promotion
            // rationale as the Coalesce arm above.
            // Non-nullable when either argument is non-nullable.
            let args = func.arguments();
            let first_type = args.first().and_then(|a| infer_expression_type(a, ctx));
            let second_type = args.get(1).and_then(|a| infer_expression_type(a, ctx));
            let data_type = promote_arg_types(func, ctx)
                .unwrap_or(DataType::Unknown(smelt_types::UnknownReason::Dynamic));
            let has_non_nullable = first_type.as_ref().is_some_and(|t| !t.nullable)
                || second_type.as_ref().is_some_and(|t| !t.nullable);
            Some(TypedColumn {
                data_type,
                nullable: !has_non_nullable,
            })
        }

        SqlFunction::RowNumber
        | SqlFunction::Rank
        | SqlFunction::DenseRank
        | SqlFunction::Ntile => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: false,
        }),

        SqlFunction::CumeDist | SqlFunction::PercentRank => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: false,
        }),

        SqlFunction::Lag
        | SqlFunction::Lead
        | SqlFunction::FirstValue
        | SqlFunction::LastValue
        | SqlFunction::NthValue => first_arg_type_or(func, ctx, DataType::unknown_dynamic(), true),

        SqlFunction::Now | SqlFunction::CurrentTimestamp => Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: true,
            },
            nullable: false,
        }),

        SqlFunction::CurrentDate => Some(TypedColumn {
            data_type: DataType::Date,
            nullable: false,
        }),

        SqlFunction::Date => Some(TypedColumn {
            data_type: DataType::Date,
            nullable: true,
        }),

        SqlFunction::DateTrunc => {
            // Mirror the tz-axis of the second argument (the timestamp).
            // DATE_TRUNC(part, ts) — argument index 1 is the timestamp.
            // Nullability propagates from the ts argument: DATE_TRUNC does not
            // introduce NULL, so it is nullable iff the input is nullable. When
            // the input type is unknown (None) we conservatively default to
            // NOT NULL rather than nullable — unknown inputs are typically NOT
            // NULL in practice and this avoids false-positive D-52 diagnostics
            // on partition columns that use date_trunc to bucket timestamps.
            let inner_type = func
                .arguments()
                .get(1)
                .and_then(|arg| infer_expression_type(arg, ctx));
            let with_timezone = inner_type
                .as_ref()
                .and_then(|tc| match tc.data_type {
                    DataType::Timestamp { with_timezone } => Some(with_timezone),
                    _ => None,
                })
                .unwrap_or(false);
            let nullable = inner_type.map(|tc| tc.nullable).unwrap_or(false);
            Some(TypedColumn {
                data_type: DataType::Timestamp { with_timezone },
                nullable,
            })
        }

        SqlFunction::Concat
        | SqlFunction::Upper
        | SqlFunction::Lower
        | SqlFunction::Md5
        | SqlFunction::Trim
        | SqlFunction::Ltrim
        | SqlFunction::Rtrim
        | SqlFunction::Substring
        | SqlFunction::Substr => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::Length | SqlFunction::CharLength | SqlFunction::CharacterLength => {
            Some(TypedColumn {
                data_type: DataType::BigInt,
                nullable: true,
            })
        }

        SqlFunction::ToChar => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::BoolAnd | SqlFunction::BoolOr | SqlFunction::Every => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: true,
        }),

        SqlFunction::Abs => {
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    return Some(arg_type);
                }
            }
            Some(TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            })
        }

        SqlFunction::Sign => Some(TypedColumn {
            data_type: DataType::SmallInt,
            nullable: true,
        }),

        SqlFunction::Round | SqlFunction::Trunc | SqlFunction::Truncate => {
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    return Some(arg_type);
                }
            }
            Some(TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            })
        }

        SqlFunction::Ceil | SqlFunction::Ceiling | SqlFunction::Floor => {
            // DuckDB: CEIL/FLOOR(DECIMAL(p,s)) → Decimal(p,0), all others → Double
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    let result_type = match &arg_type.data_type {
                        DataType::Decimal { precision, .. } => DataType::Decimal {
                            precision: *precision,
                            scale: 0,
                        },
                        _ => DataType::Double,
                    };
                    return Some(TypedColumn {
                        data_type: result_type,
                        nullable: arg_type.nullable,
                    });
                }
            }
            Some(TypedColumn {
                data_type: DataType::Double,
                nullable: true,
            })
        }

        SqlFunction::Power
        | SqlFunction::Pow
        | SqlFunction::Sqrt
        | SqlFunction::Exp
        | SqlFunction::Ln
        | SqlFunction::Log
        | SqlFunction::Log10
        | SqlFunction::Log2 => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),

        SqlFunction::Mod => {
            // Fold both operand types via promote_types rather than taking the
            // first argument's type verbatim — see the Coalesce arm above for
            // the same rationale (DuckDB widens MOD across mixed numeric types,
            // e.g. MOD(SMALLINT, BIGINT) -> BIGINT).
            let data_type = promote_arg_types(func, ctx).unwrap_or(DataType::Integer);
            Some(TypedColumn {
                data_type,
                nullable: true,
            })
        }

        SqlFunction::Sin
        | SqlFunction::Cos
        | SqlFunction::Tan
        | SqlFunction::Asin
        | SqlFunction::Acos
        | SqlFunction::Atan
        | SqlFunction::Atan2
        | SqlFunction::Sinh
        | SqlFunction::Cosh
        | SqlFunction::Tanh => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),

        SqlFunction::Pi | SqlFunction::Random => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: false,
        }),

        SqlFunction::Extract
        | SqlFunction::DatePart
        | SqlFunction::Year
        | SqlFunction::Month
        | SqlFunction::Day
        | SqlFunction::DayOfWeek
        | SqlFunction::Quarter => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: true,
        }),

        SqlFunction::MakeDate => Some(TypedColumn {
            data_type: DataType::Date,
            nullable: true,
        }),

        SqlFunction::MakeTime => Some(TypedColumn {
            data_type: DataType::Time,
            nullable: true,
        }),

        SqlFunction::MakeTimestamp => Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: false,
            },
            nullable: true,
        }),

        SqlFunction::MakeTimestamptz => Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: true,
            },
            nullable: true,
        }),

        SqlFunction::Age | SqlFunction::ToSeconds => Some(TypedColumn {
            data_type: DataType::Interval,
            nullable: true,
        }),

        SqlFunction::DateAdd | SqlFunction::DateSub => Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: false,
            },
            nullable: true,
        }),

        SqlFunction::Replace
        | SqlFunction::Translate
        | SqlFunction::Reverse
        | SqlFunction::Repeat
        | SqlFunction::Lpad
        | SqlFunction::Rpad
        | SqlFunction::Initcap
        | SqlFunction::QuoteIdent
        | SqlFunction::QuoteLiteral => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::Left | SqlFunction::Right => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::Position | SqlFunction::Strpos => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: true,
        }),

        SqlFunction::SplitPart => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::Greatest | SqlFunction::Least => {
            // Fold all argument types via promote_types rather than taking the
            // first concrete type verbatim — see the Coalesce arm above for the
            // same rationale (DuckDB itself widens GREATEST/LEAST across mixed
            // numeric argument types).
            let data_type = promote_arg_types(func, ctx)
                .unwrap_or(DataType::Unknown(smelt_types::UnknownReason::Dynamic));
            Some(TypedColumn {
                data_type,
                nullable: true,
            })
        }

        SqlFunction::ArrayAgg => {
            if let Some(arg) = func.arguments().first() {
                if let Some(arg_type) = infer_expression_type(arg, ctx) {
                    return Some(TypedColumn {
                        data_type: DataType::Array(Box::new(arg_type.data_type)),
                        nullable: true,
                    });
                }
            }
            Some(TypedColumn {
                data_type: DataType::Array(Box::new(DataType::unknown_dynamic())),
                nullable: true,
            })
        }

        SqlFunction::StringAgg | SqlFunction::Listagg => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::JsonObject
        | SqlFunction::JsonArray
        | SqlFunction::ToJson
        | SqlFunction::JsonExtract
        | SqlFunction::JsonExtractText => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::JsonArrayLength => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: true,
        }),

        SqlFunction::JsonObjectKeys => Some(TypedColumn {
            data_type: DataType::Array(Box::new(DataType::Text)),
            nullable: true,
        }),

        // json_contains is NULL-propagating per spec §11:
        // json_contains(NULL, ...) = NULL and json_contains(..., NULL) = NULL.
        SqlFunction::JsonContains => Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: true,
        }),

        // Aggregate functions from optimizer that don't have specialized type inference yet
        SqlFunction::Stddev
        | SqlFunction::Variance
        | SqlFunction::StddevPop
        | SqlFunction::StddevSamp
        | SqlFunction::VarPop
        | SqlFunction::VarSamp
        | SqlFunction::Corr
        | SqlFunction::CovarPop
        | SqlFunction::CovarSamp
        | SqlFunction::RegrSlope => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),

        SqlFunction::Median => {
            // MEDIAN interpolates: integer-family inputs widen to Double (DuckDB
            // and Spark both return DOUBLE for integer medians); Decimal/Double/
            // temporal inputs keep their own type.
            // Regression: median_integer_infers_double.
            first_arg_type_or(func, ctx, DataType::Double, true).map(|tc| match tc.data_type {
                DataType::SmallInt | DataType::Integer | DataType::BigInt => TypedColumn {
                    data_type: DataType::Double,
                    nullable: tc.nullable,
                },
                _ => tc,
            })
        }

        SqlFunction::Mode => first_arg_type_or(func, ctx, DataType::unknown_dynamic(), true),

        SqlFunction::PercentileCont | SqlFunction::PercentileDisc => Some(TypedColumn {
            data_type: DataType::Double,
            nullable: true,
        }),

        SqlFunction::ApproxCountDistinct => Some(TypedColumn {
            data_type: DataType::BigInt,
            nullable: false,
        }),

        SqlFunction::AnyValue
        | SqlFunction::ArgMax
        | SqlFunction::ArgMin
        | SqlFunction::First
        | SqlFunction::Last => first_arg_type_or(func, ctx, DataType::unknown_dynamic(), true),

        SqlFunction::GroupConcat => Some(TypedColumn {
            data_type: DataType::Text,
            nullable: true,
        }),

        SqlFunction::BitAnd | SqlFunction::BitOr | SqlFunction::BitXor => {
            first_arg_type_or(func, ctx, DataType::BigInt, true)
        }
    }
}
