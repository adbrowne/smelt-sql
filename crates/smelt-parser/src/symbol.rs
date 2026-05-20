//! Cursor-to-symbol resolution for LSP features.
//!
//! Pure function that maps a cursor offset to the symbol under the cursor.
//! Used by goto-definition, find-references, rename, and code actions.

use crate::ast::{File as AstFile, SmeltPathCall, SmeltPathRef};
use crate::syntax_kind::SyntaxKind;

/// The kind of symbol found at a cursor position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolAtCursor {
    /// Cursor is on a `smelt.<path>` path ref (e.g., `smelt.models.users`).
    /// `segments` holds the path components after the leading `smelt` token.
    PathRef { segments: Vec<String> },
    /// Cursor is on the dotted path of a `smelt.<path>(args)` call
    /// (e.g., `smelt.functions.sessionize(...)`). `segments` holds the path
    /// components after the leading `smelt` token — same shape as `PathRef`
    /// but distinguished so goto-def can route function calls to function
    /// definitions rather than to a model file.
    FunctionCall { segments: Vec<String> },
    /// Cursor is on a CTE name in a FROM/JOIN clause (reference site)
    CteReference { name: String },
    /// Cursor is on a CTE name in a WITH clause (definition site)
    CteDefinition { name: String },
    /// Cursor is on a column reference like `t.user_id` or `user_id`
    ColumnRef {
        qualifier: Option<String>,
        name: String,
    },
}

/// Resolve what symbol the cursor is on, given a parsed file and its text.
///
/// This is a pure function — no Salsa or database dependency.
/// Returns `None` if the cursor is not on a recognizable symbol.
pub fn symbol_at_cursor(file: &AstFile, _text: &str, offset: usize) -> Option<SymbolAtCursor> {
    // Check SmeltPathRef nodes FIRST — they are more specific than legacy calls.
    for node in file.syntax().descendants() {
        if node.kind() == SyntaxKind::SMELT_PATH_REF {
            if let Some(path_ref) = SmeltPathRef::cast(node) {
                let range = path_ref.text_range();
                let start: usize = range.start().into();
                let end: usize = range.end().into();
                if offset >= start && offset <= end {
                    let segments = path_ref.segments();
                    if !segments.is_empty() {
                        return Some(SymbolAtCursor::PathRef { segments });
                    }
                    return None;
                }
            }
        }
    }

    // Check SmeltPathCall nodes — cursor must be on the path portion only
    // (not arguments or PASSING clauses). Uses `call_path_range()` so the
    // hit zone is the dotted path inside the call, not the full call expr.
    for node in file.syntax().descendants() {
        if node.kind() == SyntaxKind::SMELT_PATH_CALL {
            if let Some(call) = SmeltPathCall::cast(node) {
                if let Some(path_range) = call.call_path_range() {
                    let start: usize = path_range.start().into();
                    let end: usize = path_range.end().into();
                    if offset >= start && offset <= end {
                        let segments = call.segments();
                        if !segments.is_empty() {
                            return Some(SymbolAtCursor::FunctionCall { segments });
                        }
                        return None;
                    }
                }
            }
        }
    }

    // Check CTE definitions and references
    if let Some(select_stmt) = file.select_stmt() {
        // Collect CTE definition names
        let mut cte_names: Vec<String> = Vec::new();
        if let Some(with_clause) = select_stmt.with_clause() {
            for cte in with_clause.ctes() {
                if let Some(name) = cte.name() {
                    // Check if cursor is on this CTE's name token in the definition
                    if let Some(name_range) = cte.name_range() {
                        let start: usize = name_range.start().into();
                        let end: usize = name_range.end().into();
                        if offset >= start && offset <= end {
                            return Some(SymbolAtCursor::CteDefinition { name: name.clone() });
                        }
                    }
                    cte_names.push(name);
                }
            }
        }

        // Check CTE references in FROM/JOIN clauses
        if !cte_names.is_empty() {
            if let Some(from_clause) = select_stmt.from_clause() {
                let table_refs: Vec<_> = from_clause
                    .table_refs()
                    .chain(from_clause.joins().filter_map(|j| j.table_ref()))
                    .collect();

                for table_ref in table_refs {
                    // Skip function calls and subqueries
                    if table_ref.function_call().is_some() || table_ref.subquery().is_some() {
                        continue;
                    }
                    let tr_range = table_ref.syntax().text_range();
                    let tr_start: usize = tr_range.start().into();
                    let tr_end: usize = tr_range.end().into();

                    if offset >= tr_start && offset <= tr_end {
                        if let Some(ident) = table_ref.identifier() {
                            if cte_names.contains(&ident) {
                                return Some(SymbolAtCursor::CteReference { name: ident });
                            }
                        }
                        break;
                    }
                }
            }
        }
    }

    // Check column references — find tightest expression containing cursor
    let mut best_expr: Option<crate::ast::Expr> = None;
    let mut best_len = usize::MAX;

    for node in file.syntax().descendants() {
        if let Some(expr) = crate::ast::Expr::cast(node) {
            let range = expr.text_range();
            let start: usize = range.start().into();
            let end: usize = range.end().into();
            let len = end - start;

            if offset >= start && offset <= end && len <= best_len {
                best_len = len;
                best_expr = Some(expr);
            }
        }
    }

    if let Some(expr) = best_expr {
        if let Some(col_ref) = expr.as_column_ref() {
            return Some(SymbolAtCursor::ColumnRef {
                qualifier: col_ref.qualifier().map(|s| s.to_string()),
                name: col_ref.name().to_string(),
            });
        }
    }

    None
}

/// Validate that a string is a valid SQL identifier.
///
/// Must be non-empty, start with a letter or underscore, and contain only
/// alphanumeric characters and underscores.
pub fn is_valid_sql_identifier(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Convert an LSP-style (line, character) position to a byte offset in the text.
pub fn position_to_offset(text: &str, line: u32, character: u32) -> usize {
    let mut offset = 0usize;
    let mut current_line = 0u32;
    let mut current_col = 0u32;

    for ch in text.chars() {
        if current_line == line && current_col == character {
            break;
        }
        if ch == '\n' {
            current_line += 1;
            current_col = 0;
        } else {
            current_col += 1;
        }
        offset += ch.len_utf8();
    }
    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_sql_identifier_valid() {
        assert!(is_valid_sql_identifier("foo_bar"));
        assert!(is_valid_sql_identifier("_x1"));
        assert!(is_valid_sql_identifier("CTE1"));
        assert!(is_valid_sql_identifier("a"));
        assert!(is_valid_sql_identifier("_"));
    }

    #[test]
    fn test_is_valid_sql_identifier_invalid() {
        assert!(!is_valid_sql_identifier(""));
        assert!(!is_valid_sql_identifier("1abc"));
        assert!(!is_valid_sql_identifier("a-b"));
        assert!(!is_valid_sql_identifier("has space"));
        assert!(!is_valid_sql_identifier("a.b"));
    }
}
