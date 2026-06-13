//! Type inference for `expr COLLATE collation_name` expressions (§17).
//!
//! Pure functions — no Salsa imports, no `#[salsa::tracked]`.
//!
//! The binary collation set (portable across DuckDB, Spark, Postgres) is:
//!   { "C", "POSIX", "BINARY", "UTF8_BINARY" }   (case-insensitive)
//!
//! For a binary collation, the expression type passes through unchanged.
//! For any other collation name, a `NonPortableCollation` Error is pushed at
//! the `COLLATE` clause span and the expression type degrades to `Unknown`.

use smelt_parser::ast::{CollateExpr, SelectStmt};
use smelt_types::{DataType, TypedColumn};

use super::dispatch::infer_expression_type;
use super::type_context::TypeContext;
use crate::{Diagnostic, DiagnosticCode, DiagnosticSeverity};

/// The set of binary (portable) collation names, normalised to uppercase.
const BINARY_COLLATIONS: &[&str] = &["C", "POSIX", "BINARY", "UTF8_BINARY"];

/// Returns true if `name` is a binary (portable) collation.
fn is_binary_collation(name: &str) -> bool {
    let upper = name.to_uppercase();
    BINARY_COLLATIONS.contains(&upper.as_str())
}

/// Walk all `COLLATE_EXPR` nodes in a SELECT statement and emit one
/// `NonPortableCollation` Error for each non-binary collation.
///
/// Returns the list of diagnostics. Pure — no Salsa calls.
pub fn check_collation_diagnostics(
    select_stmt: &SelectStmt,
    _ctx: &TypeContext,
) -> Vec<Diagnostic> {
    use smelt_parser::SyntaxKind::COLLATE_EXPR;

    let mut diags: Vec<Diagnostic> = Vec::new();
    let root = select_stmt.syntax();

    for node in root.descendants() {
        if node.kind() != COLLATE_EXPR {
            continue;
        }
        let collate = match CollateExpr::cast(node) {
            Some(c) => c,
            None => continue,
        };

        let collation_name = collate.collation_name();
        let is_binary = collation_name.as_deref().is_some_and(is_binary_collation);

        if !is_binary {
            let range = collate.syntax().text_range();
            let name_display = collation_name.as_deref().unwrap_or("<unknown>").to_string();
            diags.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "non-portable collation '{name_display}': only binary collations \
                     (COLLATE \"C\", COLLATE BINARY, COLLATE UTF8_BINARY, COLLATE POSIX) \
                     are valid in portable code; compare byte-wise (the default) or declare \
                     an engine on the model"
                ),
                range,
                code: Some(DiagnosticCode::NonPortableCollation),
                data: None,
            });
        }
    }

    diags
}

/// Infer the type for a `COLLATE_EXPR` without side-channel diagnostics.
///
/// Used by `infer_expression_type` in the dispatch path. Diagnostics are
/// surfaced separately by `check_collation_diagnostics` called from
/// `check_file_diagnostics` in `lib.rs`.
///
/// - Binary collation → operand type unchanged.
/// - Non-binary or missing → `Unknown` (the diagnostic fires separately).
pub fn infer_collate_expr_type(collate: &CollateExpr, ctx: &TypeContext) -> Option<TypedColumn> {
    let collation_name = collate.collation_name();
    if collation_name.as_deref().is_some_and(is_binary_collation) {
        collate
            .operand()
            .and_then(|op| infer_expression_type(&op, ctx))
    } else {
        Some(TypedColumn {
            data_type: DataType::Unknown,
            nullable: true,
        })
    }
}
