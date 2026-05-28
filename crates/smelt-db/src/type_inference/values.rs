//! Type inference for VALUES-derived tables.
//!
//! A `(VALUES (e1, e2, …), (f1, f2, …), …) AS t(c1, c2, …)` derived table
//! produces a typed schema where each column's type is the LUB (least upper
//! bound under the numeric promotion lattice) of the corresponding element
//! across all rows.
//!
//! # Pure-function rule
//!
//! Every item in this module is a pure function — no Salsa imports, no
//! `#[salsa::tracked]`.

use smelt_parser::ast::{Expr, ValuesClause};
use smelt_parser::SyntaxKind;
use smelt_types::{DataType, TypedColumn};

use super::dispatch::{infer_expression_type, promote_types};
use super::type_context::TypeContext;

/// Sentinel returned when the VALUES clause is empty (zero rows).
///
/// This is surfaced as a diagnostic at the schema-integration site (Phase 3).
/// Phase 2 callers that receive `Err(ValuesError::Empty)` must not produce
/// silent `Unknown` columns — they should treat the error as a deferred
/// diagnostic and expose it to the user in the next phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValuesError {
    /// The VALUES clause had no rows.
    Empty,
}

/// Infer column types for a VALUES clause by unifying corresponding
/// row-element types across all rows using the existing `promote_types` lattice.
///
/// Returns `Ok(types)` where `types[i]` is the LUB type of column `i` across
/// all rows, or `Err(ValuesError::Empty)` when the clause has no rows.
///
/// Column count is determined by the first row.  If subsequent rows have a
/// different number of elements (arity mismatch), they are handled gracefully:
/// extra elements beyond the first-row column count are ignored; columns with
/// no corresponding element in a later row retain their current type without
/// further promotion.
///
/// The `ctx` parameter is passed through to the expression inference entry
/// point so that column references inside VALUES row expressions can resolve.
pub fn infer_values_columns(
    values: &ValuesClause,
    ctx: &TypeContext,
) -> Result<Vec<TypedColumn>, ValuesError> {
    use SyntaxKind::VALUES_ROW;

    // Collect all rows: each row is a Vec<Expr>.
    let rows: Vec<Vec<Expr>> = values
        .syntax()
        .children()
        .filter(|n| n.kind() == VALUES_ROW)
        .map(|row_node| {
            // The VALUES_ROW children are EXPRESSION nodes produced by
            // `parse_expression()` inside the parser's loop.
            row_node.children().filter_map(Expr::cast).collect()
        })
        .collect();

    if rows.is_empty() {
        return Err(ValuesError::Empty);
    }

    let col_count = rows[0].len();
    if col_count == 0 {
        return Err(ValuesError::Empty);
    }

    // Seed the accumulator from the first row.
    let mut column_types: Vec<TypedColumn> = rows[0]
        .iter()
        .map(|expr| {
            infer_expression_type(expr, ctx).unwrap_or(TypedColumn {
                data_type: DataType::Unknown,
                nullable: true,
            })
        })
        .collect();

    // Promote across subsequent rows column-by-column.
    for row in rows.iter().skip(1) {
        for (col_idx, expr) in row.iter().take(col_count).enumerate() {
            if let Some(row_type) = infer_expression_type(expr, ctx) {
                column_types[col_idx] = promote_types(&column_types[col_idx], &row_type);
            }
            // If inference returns None (unknown expression shape), leave the
            // accumulated type unchanged — the partial knowledge from earlier
            // rows is better than discarding it.
        }
    }

    Ok(column_types)
}
