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

use rowan::TextRange;
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
                data_type: DataType::Unknown(smelt_types::UnknownReason::Dynamic),
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

    // Get the inner query body — either a SELECT statement wrapped in a
    // SUBQUERY child, or (for a VALUES-body CTE) a VALUES_CLAUSE that sits
    // directly under the CTE node with no SUBQUERY wrapper.
    let inner_count = if let Some(select_stmt) = cte.query().and_then(|q| q.select_stmt()) {
        // Count inner SELECT items; skip if wildcard SELECT.
        match select_non_wildcard_item_count(&select_stmt) {
            Some(n) => n,
            None => return out, // wildcard or empty → can't statically check
        }
    } else if let Some(values_clause) = cte.syntax().children().find_map(ValuesClause::cast) {
        match values_column_count(&values_clause) {
            Some(n) => n,
            None => return out, // empty VALUES → no static column count to compare
        }
    } else {
        return out;
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

// ─── Temporal-mix check for VALUES columns ────────────────────────────────────

/// Return a `TypeMismatch` diagnostic if `types` contains a prohibited
/// temporal mix within a single VALUES column:
/// - naive `Timestamp` and tz-aware `Timestamp WITH TIME ZONE` coexist, or
/// - `Date` and any `Timestamp` coexist.
///
/// `span` is the span of the enclosing VALUES clause (used as the error anchor).
fn check_mixed_temporal_mismatch(types: &[DataType], span: TextRange) -> Option<Diagnostic> {
    let has_naive_ts = types.iter().any(|dt| {
        matches!(
            dt,
            DataType::Timestamp {
                with_timezone: false
            }
        )
    });
    let has_tz_ts = types.iter().any(|dt| {
        matches!(
            dt,
            DataType::Timestamp {
                with_timezone: true
            }
        )
    });
    let has_date = types.iter().any(|dt| matches!(dt, DataType::Date));
    let has_timestamp = types
        .iter()
        .any(|dt| matches!(dt, DataType::Timestamp { .. }));

    if has_naive_ts && has_tz_ts {
        return Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: "Timezone mismatch in VALUES: column mixes naive Timestamp and \
                      Timestamp WITH TIME ZONE; add an explicit CAST to align timezone variants"
                .to_string(),
            range: span,
            code: Some(DiagnosticCode::TypeMismatch),
            data: None,
        });
    }
    if has_date && has_timestamp {
        return Some(Diagnostic {
            severity: DiagnosticSeverity::Error,
            message: "Type mismatch in VALUES: column mixes Date and Timestamp types; \
                      add an explicit CAST to align temporal types"
                .to_string(),
            range: span,
            code: Some(DiagnosticCode::TypeMismatch),
            data: None,
        });
    }
    None
}

/// Walk all VALUES-derived-table subqueries in a SELECT statement and emit
/// a `TypeMismatch` diagnostic for each column that contains a prohibited
/// temporal mix (strict §16 rule: naive/tz-aware or Date/Timestamp).
///
/// This function is PURE — no Salsa calls.
pub fn check_mixed_temporal_values_diagnostics(
    select_stmt: &SelectStmt,
    ctx: &TypeContext,
) -> Vec<Diagnostic> {
    use smelt_parser::SyntaxKind::TABLE_REF;

    let mut diags: Vec<Diagnostic> = Vec::new();

    for node in select_stmt.syntax().descendants() {
        if node.kind() != TABLE_REF {
            continue;
        }
        let table_ref = match TableRef::cast(node) {
            Some(t) => t,
            None => continue,
        };
        let subquery = match table_ref.subquery() {
            Some(s) => s,
            None => continue,
        };
        let values_clause = match subquery.values_clause() {
            Some(v) => v,
            None => continue,
        };

        let values_span = values_clause.syntax().text_range();

        // Re-collect rows (mirrors infer_values_columns logic without the promotion step).
        let rows: Vec<Vec<Expr>> = values_clause
            .syntax()
            .children()
            .filter(|n| n.kind() == SyntaxKind::VALUES_ROW)
            .map(|row_node| row_node.children().filter_map(Expr::cast).collect())
            .collect();

        let col_count = rows.first().map(|r| r.len()).unwrap_or(0);
        if col_count == 0 {
            continue;
        }

        for col_idx in 0..col_count {
            let col_types: Vec<DataType> = rows
                .iter()
                .filter_map(|row| row.get(col_idx))
                .filter_map(|expr| infer_expression_type(expr, ctx))
                .filter(|tc| !matches!(tc.data_type, DataType::Unknown(_) | DataType::Null))
                .map(|tc| tc.data_type)
                .collect();

            if let Some(diag) = check_mixed_temporal_mismatch(&col_types, values_span) {
                diags.push(diag);
            }
        }
    }

    diags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics_types::DiagnosticCode;

    fn parse_select(sql: &str) -> smelt_parser::ast::SelectStmt {
        use smelt_parser::ast::File;
        let parse = smelt_parser::parse(sql);
        let root = parse.syntax();
        let file = File::cast(root).expect("failed to cast to File");
        file.select_stmt().expect("no SelectStmt in parsed SQL")
    }

    #[test]
    fn values_mixed_tz_is_type_mismatch() {
        // A VALUES column that mixes naive Timestamp and TIMESTAMPTZ must produce
        // exactly one TypeMismatch diagnostic (not silent Unknown).
        let sql = "SELECT * FROM \
            (VALUES (TIMESTAMP '2020-01-01 00:00:00'), (TIMESTAMPTZ '2020-01-01 00:00:00+00')) \
            AS v(ts)";
        let select = parse_select(sql);
        let ctx = TypeContext::new();
        let diags = check_mixed_temporal_values_diagnostics(&select, &ctx);
        assert!(
            !diags.is_empty(),
            "expected a TypeMismatch diagnostic for mixed tz VALUES column, got none"
        );
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(DiagnosticCode::TypeMismatch)),
            "diagnostic must have TypeMismatch code, got: {diags:?}"
        );
    }

    #[test]
    fn values_date_timestamp_mix_is_type_mismatch() {
        // A VALUES column that mixes Date and Timestamp must produce a TypeMismatch.
        let sql = "SELECT * FROM \
            (VALUES (DATE '2020-01-01'), (TIMESTAMP '2020-01-01 00:00:00')) \
            AS v(dt)";
        let select = parse_select(sql);
        let ctx = TypeContext::new();
        let diags = check_mixed_temporal_values_diagnostics(&select, &ctx);
        assert!(
            !diags.is_empty(),
            "expected a TypeMismatch diagnostic for Date/Timestamp VALUES mix, got none"
        );
        assert!(
            diags
                .iter()
                .any(|d| d.code == Some(DiagnosticCode::TypeMismatch)),
            "diagnostic must have TypeMismatch code, got: {diags:?}"
        );
    }

    #[test]
    fn values_homogeneous_temporal_ok() {
        // All-naive timestamps: no diagnostic.
        let sql = "SELECT * FROM \
            (VALUES (TIMESTAMP '2020-01-01 00:00:00'), (TIMESTAMP '2020-06-15 12:00:00')) \
            AS v(ts)";
        let select = parse_select(sql);
        let ctx = TypeContext::new();
        let diags = check_mixed_temporal_values_diagnostics(&select, &ctx);
        assert!(
            diags.is_empty(),
            "homogeneous naive-Timestamp VALUES column must not produce diagnostics, \
             got: {diags:?}"
        );

        // All tz-aware timestamps: no diagnostic.
        let sql2 = "SELECT * FROM \
            (VALUES (TIMESTAMPTZ '2020-01-01 00:00:00+00'), (TIMESTAMPTZ '2020-06-15 12:00:00+01')) \
            AS v(ts)";
        let select2 = parse_select(sql2);
        let diags2 = check_mixed_temporal_values_diagnostics(&select2, &ctx);
        assert!(
            diags2.is_empty(),
            "homogeneous tz-aware-Timestamp VALUES column must not produce diagnostics, \
             got: {diags2:?}"
        );
    }
}
