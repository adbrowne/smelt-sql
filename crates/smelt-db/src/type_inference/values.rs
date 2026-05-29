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

use smelt_parser::ast::{Cte, Expr, SelectItem, SelectStmt, TableRef, ValuesClause};
use smelt_parser::SyntaxKind;
use smelt_types::{DataType, TypedColumn};

use crate::diagnostics_types::{Diagnostic, DiagnosticCode, DiagnosticSeverity};

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

// ─── Arity-mismatch and empty-VALUES diagnostic checks ───────────────────────

/// Count the number of VALUES rows' first row column count.
/// Returns `None` when the VALUES clause has no rows.
pub fn values_column_count(values: &ValuesClause) -> Option<usize> {
    use SyntaxKind::VALUES_ROW;
    let first_row = values
        .syntax()
        .children()
        .find(|n| n.kind() == VALUES_ROW)?;
    let count = first_row.children().filter_map(Expr::cast).count();
    if count == 0 {
        None
    } else {
        Some(count)
    }
}

/// Count the non-wildcard SELECT items in a SELECT statement.
/// Returns `None` when there are no items or when any item is a wildcard
/// (`SELECT *`), because wildcard counts depend on upstream schema.
pub fn select_non_wildcard_item_count(select: &smelt_parser::ast::SelectStmt) -> Option<usize> {
    let list = select.select_list()?;
    let items: Vec<SelectItem> = list.items().collect();
    if items.is_empty() {
        return None;
    }
    // If any item is a wildcard, we can't count statically.
    if items.iter().any(|item| item.is_wildcard()) {
        return None;
    }
    Some(items.len())
}

/// Check a single `TABLE_REF` node for VALUES derived-table arity mismatches
/// and emit zero or one `AliasColumnArityMismatch` or `EmptyValuesClause`
/// diagnostics.
///
/// Returns a `Vec<Diagnostic>` (empty when no issue is found).
pub fn check_table_ref_values_arity(table_ref: &TableRef) -> Vec<Diagnostic> {
    use smelt_parser::SyntaxKind::ALIAS_COLUMN_LIST;
    let mut out = Vec::new();

    let subquery = match table_ref.subquery() {
        Some(s) => s,
        None => return out,
    };

    let values_clause = match subquery.values_clause() {
        Some(v) => v,
        None => return out,
    };

    // Empty VALUES check: zero rows → EmptyValuesClause.
    let col_count = match values_column_count(&values_clause) {
        Some(n) => n,
        None => {
            // values_column_count returns None for zero rows.
            let range = values_clause.syntax().text_range();
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: "VALUES clause has no rows; cannot infer column types".to_string(),
                range,
                code: Some(DiagnosticCode::EmptyValuesClause),
                data: None,
            });
            return out;
        }
    };

    // Arity check: only when an explicit alias column list is present.
    let alias_names = match table_ref.alias_column_names() {
        Some(names) => names,
        None => return out, // no alias list → no check
    };

    if alias_names.len() == col_count {
        return out; // matching → no diagnostic
    }

    // Mismatch: anchor at the ALIAS_COLUMN_LIST span.
    let acl_node = table_ref
        .syntax()
        .children()
        .find(|n| n.kind() == ALIAS_COLUMN_LIST);
    let range = if let Some(acl) = acl_node {
        acl.text_range()
    } else {
        // Fallback to the whole TABLE_REF span.
        table_ref.syntax().text_range()
    };

    out.push(Diagnostic {
        severity: DiagnosticSeverity::Error,
        message: format!(
            "alias column list has {} name(s) but the relation has {} column(s)",
            alias_names.len(),
            col_count
        ),
        range,
        code: Some(DiagnosticCode::AliasColumnArityMismatch),
        data: None,
    });
    out
}

/// Check a single `CTE` node for alias-column-list arity mismatches.
///
/// Returns a `Vec<Diagnostic>` (empty when no issue is found).
pub fn check_cte_alias_arity(cte: &Cte) -> Vec<Diagnostic> {
    use smelt_parser::SyntaxKind::ALIAS_COLUMN_LIST;
    let mut out = Vec::new();

    let explicit_names = cte.column_names();
    if explicit_names.is_empty() {
        return out; // no column list declared → no check
    }

    // Get the inner SELECT statement.
    let select_stmt: SelectStmt = match cte.query().and_then(|q| q.select_stmt()) {
        Some(s) => s,
        None => return out, // no SELECT body (e.g. VALUES CTE) → skip
    };

    // Count inner SELECT items; skip if wildcard SELECT.
    let inner_count = match select_non_wildcard_item_count(&select_stmt) {
        Some(n) => n,
        None => return out, // wildcard or empty → can't statically check
    };

    if explicit_names.len() == inner_count {
        return out; // matching → no diagnostic
    }

    // Mismatch: anchor at the ALIAS_COLUMN_LIST span.
    let acl_node = cte
        .syntax()
        .children()
        .find(|n| n.kind() == ALIAS_COLUMN_LIST);
    let range = if let Some(acl) = acl_node {
        acl.text_range()
    } else {
        // Fallback to the CTE name range.
        cte.name_range()
            .unwrap_or_else(|| cte.syntax().text_range())
    };

    out.push(Diagnostic {
        severity: DiagnosticSeverity::Error,
        message: format!(
            "alias column list has {} name(s) but the relation has {} column(s)",
            explicit_names.len(),
            inner_count
        ),
        range,
        code: Some(DiagnosticCode::AliasColumnArityMismatch),
        data: None,
    });
    out
}
