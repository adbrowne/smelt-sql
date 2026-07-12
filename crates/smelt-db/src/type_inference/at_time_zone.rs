//! Type inference for `expr AT TIME ZONE tz_expr` (timezone conversion).
//!
//! Pure functions — no Salsa imports, no `#[salsa::tracked]`.
//!
//! DuckDB semantics (verified against the DuckDB oracle):
//! - `TIMESTAMP AT TIME ZONE tz` → `TIMESTAMP WITH TIME ZONE`
//! - `TIMESTAMP WITH TIME ZONE AT TIME ZONE tz` → `TIMESTAMP` (plain, naive)
//!
//! i.e. the operator toggles `with_timezone` on a `DataType::Timestamp`
//! operand. Nullability propagates from the operand (the operator itself
//! never introduces NULL).
//!
//! DuckDB also accepts a `DATE` operand (implicitly cast to `TIMESTAMP` first)
//! but rejects `TIME`/`TIME WITH TIME ZONE` and non-temporal operands
//! (verified via the oracle: `timezone(VARCHAR, TIME)` has no matching
//! overload). Only the `Timestamp{with_timezone}` round-trip is implemented
//! here; any other operand type degrades to `Unknown(Unresolved)`.

use smelt_parser::ast::AtTimeZoneExpr;
use smelt_types::{DataType, TypedColumn, UnknownReason};

use super::dispatch::infer_expression_type;
use super::type_context::TypeContext;

/// Infer the type of an `AT_TIME_ZONE_EXPR` node.
pub fn infer_at_time_zone_type(at_tz: &AtTimeZoneExpr, ctx: &TypeContext) -> Option<TypedColumn> {
    let operand_type = at_tz
        .operand()
        .and_then(|op| infer_expression_type(&op, ctx));

    // The operator never introduces NULL on its own; nullability propagates
    // from the operand. An operand whose type could not be inferred at all
    // (None) defaults to nullable=true — the conservative choice, mirroring
    // `infer_extract_type`'s and `infer_collate_expr_type`'s degrade path.
    let nullable = operand_type.as_ref().map(|t| t.nullable).unwrap_or(true);

    let data_type = match operand_type.as_ref().map(|t| &t.data_type) {
        Some(DataType::Timestamp { with_timezone }) => DataType::Timestamp {
            with_timezone: !with_timezone,
        },
        _ => DataType::Unknown(UnknownReason::Unresolved),
    };

    Some(TypedColumn {
        data_type,
        nullable,
    })
}
