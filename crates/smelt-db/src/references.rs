//! Pure functions for finding references across the project.
//!
//! These functions scan pre-collected data (refs, sources) to find all
//! locations where a given symbol is referenced. They are pure functions
//! with no Salsa dependency — Salsa queries in lib.rs wrap them.

use std::path::PathBuf;

use rowan::TextRange;
use smelt_parser::ast::{File as AstFile, Range, SmeltPathCall};
use smelt_parser::SyntaxKind::{DOT, IDENT};

use crate::{RefLocation, SourceLocation};

/// Find all files that reference a given model name.
///
/// Returns (file_path, range) pairs for each `smelt.models.model_name` call
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
/// Returns (file_path, range) pairs for each `smelt.sources.source.table` call
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

/// Find all `smelt.functions.<name>(...)` call sites in a single file.
///
/// Returns the `SMELT_PATH` text range (the dotted path, e.g. the entire
/// `smelt.functions.sessionize`) for each call whose segments equal
/// `["functions", function_name]`. Callers map these ranges to LSP `Location`s
/// and union across the project's files; see `architecture.md` → "Project
/// isolation rule" for why find-references is project-scoped.
pub fn find_function_call_sites_in_file(file: &AstFile, function_name: &str) -> Vec<TextRange> {
    let mut results = Vec::new();
    for node in file.syntax().descendants() {
        let Some(call) = SmeltPathCall::cast(node) else {
            continue;
        };
        let segs = call.segments();
        if segs.len() == 2 && segs[0] == "functions" && segs[1] == function_name {
            if let Some(range) = call.call_path_range() {
                results.push(range);
            }
        }
    }
    results
}

/// A column reference found in a file, with the text range of the column name IDENT token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnRefLocation {
    /// The text range of the column name IDENT token
    pub name_range: TextRange,
    /// The text range of the qualifier IDENT token, if present (e.g., "t" in "t.user_id")
    pub qualifier_range: Option<TextRange>,
    /// The qualifier string, if present
    pub qualifier: Option<String>,
}

/// Find all references to a column name within a single file.
///
/// Scans all IDENT tokens in the file's descendant expressions.
/// For qualified references (e.g., `t.user_id`), the qualifier must match if provided.
/// For unqualified references, matches any occurrence of the column name.
///
/// Returns `ColumnRefLocation` for each matching column reference.
pub fn find_column_references_in_file(
    file: &AstFile,
    column_name: &str,
    qualifier_filter: Option<&str>,
) -> Vec<ColumnRefLocation> {
    let mut results = Vec::new();

    for node in file.syntax().descendants() {
        if let Some(expr) = smelt_parser::ast::Expr::cast(node) {
            if let Some(col_ref) = expr.as_column_ref() {
                if col_ref.name() != column_name {
                    continue;
                }

                // Apply qualifier filter if provided
                if let Some(filter) = qualifier_filter {
                    if col_ref.qualifier() != Some(filter) {
                        continue;
                    }
                }

                // Find the column name IDENT token (the last IDENT after a DOT, or the only IDENT)
                let tokens: Vec<_> = expr
                    .syntax()
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .filter(|t| t.kind() == IDENT || t.kind() == DOT)
                    .collect();

                let (name_range, qualifier_range) = if tokens.len() >= 3
                    && tokens[0].kind() == IDENT
                    && tokens[1].kind() == DOT
                    && tokens[2].kind() == IDENT
                {
                    // Qualified: table.column
                    (tokens[2].text_range(), Some(tokens[0].text_range()))
                } else if tokens.len() == 1 && tokens[0].kind() == IDENT {
                    // Unqualified: column
                    (tokens[0].text_range(), None)
                } else {
                    continue;
                };

                results.push(ColumnRefLocation {
                    name_range,
                    qualifier_range,
                    qualifier: qualifier_range.map(|_| col_ref.qualifier().unwrap().to_string()),
                });
            }
        }
    }

    results
}

/// Find the column definition site in a SELECT list.
///
/// Returns the TextRange of the column name or alias in the SELECT list
/// where a column with the given name is defined.
pub fn find_column_definition_in_select(file: &AstFile, column_name: &str) -> Option<TextRange> {
    let select_stmt = file.select_stmt()?;
    let select_list = select_stmt.select_list()?;

    for item in select_list.items() {
        if item.column_name().as_deref() == Some(column_name) {
            // If there's an alias, return the alias range
            if let Some(alias_range) = item.alias_range() {
                return Some(alias_range);
            }
            // Otherwise if it's a simple column ref, return the IDENT token's range
            // (not the expression's range, which may include trailing whitespace)
            if let Some(expr) = item.expression() {
                if let Some(col_ref) = expr.as_column_ref() {
                    // For qualified refs (t.col), return the column name token
                    // For unqualified refs (col), return the single IDENT token
                    let tokens: Vec<_> = expr
                        .syntax()
                        .children_with_tokens()
                        .filter_map(|e| e.into_token())
                        .filter(|t| t.kind() == IDENT || t.kind() == DOT)
                        .collect();
                    let name_token = if col_ref.qualifier().is_some() && tokens.len() >= 3 {
                        Some(&tokens[2]) // qualified: table.column
                    } else if !tokens.is_empty() {
                        Some(&tokens[0]) // unqualified
                    } else {
                        None
                    };
                    if let Some(tok) = name_token {
                        return Some(tok.text_range());
                    }
                }
            }
        }
    }
    None
}
