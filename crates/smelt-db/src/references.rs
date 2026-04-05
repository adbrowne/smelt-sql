//! Pure functions for finding references across the project.
//!
//! These functions scan pre-collected data (refs, sources) to find all
//! locations where a given symbol is referenced. They are pure functions
//! with no Salsa dependency — Salsa queries in lib.rs wrap them.

use std::path::PathBuf;

use rowan::TextRange;
use smelt_parser::ast::{File as AstFile, Range};
use smelt_parser::SyntaxKind::IDENT;

use crate::{RefLocation, SourceLocation};

/// Find all files that reference a given model name.
///
/// Returns (file_path, range) pairs for each `smelt.ref('model_name')` call
/// that matches the given name.
pub fn find_model_references(
    model_name: &str,
    file_refs: &[(PathBuf, Vec<RefLocation>)],
) -> Vec<(PathBuf, Range)> {
    let mut results = Vec::new();
    for (path, refs) in file_refs {
        for ref_loc in refs {
            if ref_loc.name == model_name {
                results.push((path.clone(), ref_loc.range));
            }
        }
    }
    results
}

/// Find all files that reference a given source by qualified name (e.g., "raw.users").
///
/// Returns (file_path, range) pairs for each `smelt.source('source.table')` call
/// that matches the given qualified name.
pub fn find_source_references(
    qualified_name: &str,
    file_sources: &[(PathBuf, Vec<SourceLocation>)],
) -> Vec<(PathBuf, Range)> {
    let mut results = Vec::new();
    for (path, sources) in file_sources {
        for source_loc in sources {
            if source_loc.qualified_name == qualified_name {
                results.push((path.clone(), source_loc.range));
            }
        }
    }
    results
}

/// Find all references to a CTE within a single file, including the definition site.
///
/// Returns TextRanges for:
/// - The CTE name in the WITH clause (definition)
/// - The CTE name in FROM/JOIN clauses (references)
/// - The CTE name used as a column qualifier (e.g., `cte.col`)
pub fn find_cte_references(file: &AstFile, _text: &str, cte_name: &str) -> Vec<TextRange> {
    let mut results = Vec::new();

    let select_stmt = match file.select_stmt() {
        Some(s) => s,
        None => return results,
    };

    // 1. Find CTE definition site in WITH clause
    if let Some(with_clause) = select_stmt.with_clause() {
        for cte in with_clause.ctes() {
            if cte.name().as_deref() == Some(cte_name) {
                if let Some(name_range) = cte.name_range() {
                    results.push(name_range);
                }
            }
        }
    }

    // 2. Find CTE references in FROM/JOIN clauses
    if let Some(from_clause) = select_stmt.from_clause() {
        // Table refs in FROM
        for table_ref in from_clause.table_refs() {
            if table_ref.function_call().is_some() || table_ref.subquery().is_some() {
                continue;
            }
            if table_ref.identifier().as_deref() == Some(cte_name) {
                // Use the identifier token range from the table_ref
                for token in table_ref.syntax().children_with_tokens() {
                    if let Some(t) = token.as_token() {
                        if t.kind() == IDENT && t.text() == cte_name {
                            results.push(t.text_range());
                            break;
                        }
                    }
                }
            }
        }

        // Table refs in JOIN clauses
        for join in from_clause.joins() {
            if let Some(table_ref) = join.table_ref() {
                if table_ref.function_call().is_some() || table_ref.subquery().is_some() {
                    continue;
                }
                if table_ref.identifier().as_deref() == Some(cte_name) {
                    for token in table_ref.syntax().children_with_tokens() {
                        if let Some(t) = token.as_token() {
                            if t.kind() == IDENT && t.text() == cte_name {
                                results.push(t.text_range());
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    // 3. Find CTE name used as column qualifier (e.g., `cte.col`)
    for node in file.syntax().descendants() {
        if let Some(expr) = smelt_parser::ast::Expr::cast(node) {
            if let Some(col_ref) = expr.as_column_ref() {
                if col_ref.qualifier() == Some(cte_name) {
                    // Find the qualifier IDENT token (first IDENT in this expression)
                    for token in expr.syntax().children_with_tokens() {
                        if let Some(t) = token.as_token() {
                            if t.kind() == IDENT && t.text() == cte_name {
                                let range = t.text_range();
                                // Avoid duplicates
                                if !results.contains(&range) {
                                    results.push(range);
                                }
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    results
}
