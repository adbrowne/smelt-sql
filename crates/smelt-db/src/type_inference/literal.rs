//! Literal, cast, extract, and CASE expression type inference.

#![allow(unused_imports)]
use rowan::TextRange;
use smelt_parser::ast::{
    BinaryExpr, CaseExpr, CastExpr, Cte, Expr, ExtractExpr, FunctionCall, RowConstructor,
    SelectStmt, SmeltAsStructCall, SmeltPathCall, StructLiteral, Subquery,
};
use smelt_types::signatures::{
    kind_ceiling, unify_call_with_expected, BuiltinRegistry, ExprKind, FunctionSig, RecordRegistry,
    SmeltType, TypeConstraint,
};
use smelt_types::{parse_type, DataType, SqlFunction, TypedColumn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::type_context::TypeContext;
#[allow(unused_imports)]
use super::*;

/// Infer the type of a CAST expression.
/// Preserves nullability from the input expression: if the input is non-nullable,
/// the CAST result is also non-nullable.
pub fn infer_cast_type(cast_expr: &CastExpr, ctx: &TypeContext) -> Option<TypedColumn> {
    let type_spec = cast_expr.type_spec()?;
    let type_text = type_spec.full_text();

    // Parse the type specification; an unrecognised type returns None here, but
    // collect_expression_type_diagnostics in check_types.rs independently calls
    // parse_type and emits UnknownCastType — the error is reported there.
    // intentionally ignored: returning None propagates Unknown; the diagnostic
    // path in check_types.rs covers the user-visible error.
    let data_type = parse_type(&type_text).ok()?;

    // Normalize FLOAT to DOUBLE: DuckDB treats FLOAT as a 4-byte float but
    // smelt normalizes to DOUBLE to avoid spurious type mismatches downstream.
    let data_type = match data_type {
        DataType::Float => DataType::Double,
        other => other,
    };

    // TRY_CAST (DuckDB error-tolerant cast) returns NULL on a failed conversion,
    // so its result is ALWAYS nullable regardless of the input's nullability.
    // A plain CAST only passes NULL through from the input.
    let nullable = if cast_expr.is_try_cast() {
        true
    } else {
        // Propagate nullability from the input expression. CAST does not introduce
        // NULL — it only passes through NULL from the input. When the input type is
        // unknown (None), we default to NOT NULL rather than conservatively assuming
        // nullable, because unknown inputs are typically NOT NULL in practice (e.g.
        // columns from external sources / seeds) and the conservative default caused
        // false-positive D-52 diagnostics on common patterns like
        //   SELECT CAST(ts AS DATE) AS partition_date FROM smelt.sources.raw.events
        // Genuine nullable inputs (e.g. CASE without ELSE, outer-join columns) still
        // propagate nullable=true correctly through this path.
        cast_expr
            .expression()
            .and_then(|e| infer_expression_type(&e, ctx))
            .map(|t| t.nullable)
            .unwrap_or(false)
    };

    Some(TypedColumn {
        data_type,
        nullable,
    })
}

/// Infer the type of an EXTRACT(field FROM expr) expression.
pub fn infer_extract_type(extract_expr: &ExtractExpr) -> Option<TypedColumn> {
    let field = extract_expr.field_name().unwrap_or_default();
    let data_type = match field.as_str() {
        "EPOCH" => DataType::Double,
        "YEAR" | "MONTH" | "DAY" | "HOUR" | "MINUTE" | "SECOND" | "DOW" | "DOY" | "QUARTER"
        | "WEEK" | "DAYOFWEEK" | "DAYOFYEAR" | "ISODOW" | "ISOYEAR" | "MICROSECOND"
        | "MICROSECONDS" | "MILLISECOND" | "MILLISECONDS" | "TIMEZONE" | "TIMEZONE_HOUR"
        | "TIMEZONE_MINUTE" => DataType::BigInt,
        _ => DataType::BigInt, // default for unknown fields
    };
    Some(TypedColumn {
        data_type,
        nullable: true,
    })
}

/// Infer the type of a CASE expression.
/// The result type is the type of the first THEN expression (or ELSE if no WHEN clauses).
/// Non-nullable only when an ELSE clause is present AND all branches (THEN + ELSE) are non-nullable.
/// Without ELSE, the implicit default is NULL, making the result always nullable.
pub fn infer_case_expr_type(case_expr: &CaseExpr, ctx: &TypeContext) -> Option<TypedColumn> {
    let has_else = case_expr.else_expr().is_some();

    // Collect types from all WHEN/THEN branches, promoting across all of them
    let mut accumulated: Option<TypedColumn> = None;
    let mut all_branches_non_nullable = true;

    let merge = |acc: Option<TypedColumn>, branch: TypedColumn| -> Option<TypedColumn> {
        match acc {
            None => Some(branch),
            Some(existing) => Some(promote_types(&existing, &branch)),
        }
    };

    for when_clause in case_expr.when_clauses() {
        if let Some(result_expr) = when_clause.result() {
            if let Some(result_type) = infer_expression_type(&result_expr, ctx) {
                if result_type.nullable {
                    all_branches_non_nullable = false;
                }
                if !matches!(result_type.data_type, DataType::Unknown(_) | DataType::Null) {
                    accumulated = merge(accumulated, result_type);
                }
            } else {
                all_branches_non_nullable = false;
            }
        } else {
            all_branches_non_nullable = false;
        }
    }

    // Check ELSE branch
    if let Some(else_expr) = case_expr.else_expr() {
        if let Some(else_type) = infer_expression_type(&else_expr, ctx) {
            if else_type.nullable {
                all_branches_non_nullable = false;
            }
            if !matches!(else_type.data_type, DataType::Unknown(_) | DataType::Null) {
                accumulated = merge(accumulated, else_type);
            }
        } else {
            all_branches_non_nullable = false;
        }
    }

    let accumulated = accumulated?;
    let data_type = accumulated.data_type;

    // Non-nullable only when ELSE is present and all branches are non-nullable.
    // Without ELSE, the implicit default is NULL.
    let nullable = !(has_else && all_branches_non_nullable);

    Some(TypedColumn {
        data_type,
        nullable,
    })
}

/// Infer the type of a literal value
pub fn infer_literal_type(text: &str) -> Option<TypedColumn> {
    let text = text.trim();

    // NULL literal
    if text.eq_ignore_ascii_case("NULL") {
        return Some(TypedColumn {
            data_type: DataType::Null,
            nullable: true,
        });
    }

    // Boolean literals
    if text.eq_ignore_ascii_case("TRUE") || text.eq_ignore_ascii_case("FALSE") {
        return Some(TypedColumn {
            data_type: DataType::Boolean,
            nullable: false,
        });
    }

    // String literals (single or double quoted)
    if (text.starts_with('\'') && text.ends_with('\''))
        || (text.starts_with('"') && text.ends_with('"'))
    {
        return Some(TypedColumn {
            data_type: DataType::Text,
            nullable: false,
        });
    }

    // Dollar-quoted string literals (`$$...$$` or `$tag$...$tag$`). The
    // lexer (`crates/smelt-parser/src/lexer.rs::try_dollar_quote`) only ever
    // emits the STRING token kind for a well-formed dollar-quote whose
    // opening and closing delimiters match exactly, so text starting and
    // ending with `$` reaching this point is exactly such a literal — same
    // Text inference as an ordinary quoted string.
    if text.len() >= 2 && text.starts_with('$') && text.ends_with('$') {
        return Some(TypedColumn {
            data_type: DataType::Text,
            nullable: false,
        });
    }

    // Numeric literals
    if let Some(num_type) = infer_numeric_literal_type(text) {
        return Some(TypedColumn {
            data_type: num_type,
            nullable: false,
        });
    }

    // Niladic keyword-functions that the parser treats as bare identifiers
    // (no parentheses in standard SQL, so `expr.as_function_call()` returns None).
    // §16: CURRENT_TIMESTAMP returns Timestamp WITH TIME ZONE (non-nullable).
    // CURRENT_DATE returns Date (non-nullable) — already handled in function_call.rs
    // for the parenthesised form; replicated here for the bare-keyword form.
    let upper = text.to_uppercase();
    if upper == "CURRENT_TIMESTAMP" {
        return Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: true,
            },
            nullable: false,
        });
    }
    if upper == "CURRENT_DATE" {
        return Some(TypedColumn {
            data_type: DataType::Date,
            nullable: false,
        });
    }
    if upper.starts_with("DATE ") || upper.starts_with("DATE'") {
        return Some(TypedColumn {
            data_type: DataType::Date,
            nullable: false,
        });
    }
    if upper.starts_with("TIMESTAMPTZ ") || upper.starts_with("TIMESTAMPTZ'") {
        return Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: true,
            },
            nullable: false,
        });
    }
    if upper.starts_with("TIMESTAMP ") || upper.starts_with("TIMESTAMP'") {
        return Some(TypedColumn {
            data_type: DataType::Timestamp {
                with_timezone: false,
            },
            nullable: false,
        });
    }
    if upper.starts_with("TIME ") || upper.starts_with("TIME'") {
        return Some(TypedColumn {
            data_type: DataType::Time,
            nullable: false,
        });
    }
    if upper.starts_with("INTERVAL ") || upper.starts_with("INTERVAL'") {
        return Some(TypedColumn {
            data_type: DataType::Interval,
            nullable: false,
        });
    }

    None
}

/// Infer the type of a numeric literal
pub fn infer_numeric_literal_type(text: &str) -> Option<DataType> {
    // Strip DuckDB-style underscore digit separators (`1_000_000`,
    // `1_000.000_1`) before value-parsing — the lexer already validated
    // separator placement (strictly between two digits, never leading,
    // trailing, or doubled), so it's always safe to drop them here. The
    // stripped text is also what feeds the precision/scale digit counts
    // below, so `1_000.000_1` still yields DECIMAL(8,4) (8 total digits),
    // matching DuckDB.
    let owned;
    let text = if text.contains('_') {
        owned = text.replace('_', "");
        owned.as_str()
    } else {
        text
    };

    // Check for decimal point
    if text.contains('.') {
        // Could be DECIMAL or DOUBLE
        // If it has 'e' or 'E', it's a floating point
        if text.contains('e') || text.contains('E') {
            return Some(DataType::Double);
        }

        // Count digits for precision/scale
        let parts: Vec<&str> = text.split('.').collect();
        if parts.len() == 2 {
            let precision = parts[0].trim_start_matches('-').len() + parts[1].len();
            let scale = parts[1].len();
            return Some(DataType::Decimal {
                precision: precision.min(38) as u8,
                scale: scale.min(38) as u8,
            });
        }

        return Some(DataType::Double);
    }

    // Integer literal - check range
    if let Ok(n) = text.parse::<i64>() {
        return Some(if n >= i16::MIN as i64 && n <= i16::MAX as i64 {
            DataType::SmallInt
        } else if n >= i32::MIN as i64 && n <= i32::MAX as i64 {
            DataType::Integer
        } else {
            DataType::BigInt
        });
    }

    // Try parsing as unsigned for very large numbers
    if text.parse::<u64>().is_ok() {
        return Some(DataType::BigInt);
    }

    None
}
