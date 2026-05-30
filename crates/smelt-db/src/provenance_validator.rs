//! Phase 51 — pure `provenance:` / `joins:` validator.
//!
//! This module contains the pure analysis functions that validate a
//! `smelt.define` body against its frontmatter `provenance:` and `joins:`
//! declarations. These run only when `unstable_schema: true` in `smelt.yml`.
//!
//! # Architecture note
//! Following the pure function rule: no Salsa calls inside this module.
//! All inputs are passed as plain data structures. The Salsa wrapper in
//! `lib.rs` gathers the necessary data and calls these pure functions.

use std::collections::{HashMap, HashSet};

use smelt_parser::ast::{ColumnRef, Expr, SelectStmt};
use smelt_parser::syntax_kind::SyntaxNode;
use smelt_planner::logical::{JoinSpec, Provenance};

use crate::{Diagnostic, DiagnosticCode, DiagnosticSeverity};

/// Collect all column-reference names (recursively) from an expression subtree.
///
/// Walks all descendant nodes; for each EXPR node that `ColumnRef::from_expr`
/// recognises, records `(qualifier, name)`.
///
/// For an expression like `revenue - cost`:
/// - The outer EXPR wraps a BINARY_EXPR
/// - `root.descendants()` yields root, BINARY_EXPR, inner EXPR(revenue), inner EXPR(cost)
/// - `Expr::cast` only matches EXPR nodes (and other expression-kind nodes)
/// - `ColumnRef::from_expr` succeeds on simple leaf EXPR nodes
pub fn collect_expr_column_refs(root: &SyntaxNode) -> Vec<(Option<String>, String)> {
    let mut result = Vec::new();
    for node in root.descendants() {
        if let Some(expr) = Expr::cast(node) {
            if let Some(col) = ColumnRef::from_expr(&expr) {
                result.push((
                    col.qualifier().map(|s| s.to_string()),
                    col.name().to_string(),
                ));
            }
        }
    }
    result
}

/// Validate declared `provenance:` entries against the body's outermost SELECT.
///
/// For each `(output_col, source_cols)` in `Provenance::Declared`:
/// - Finds the SELECT item with alias matching `output_col`
/// - Collects the actual column names read by that item's expression
/// - Emits `ProvenanceMismatch` for columns declared but not read, and
///   for columns read but not declared
///
/// Column name matching strips qualifiers from both sides (compares only
/// the bare column name portion).
pub fn validate_provenance(
    select: &SelectStmt,
    provenance: &Provenance,
    anchor_range: rowan::TextRange,
) -> Vec<Diagnostic> {
    let declared_entries = match provenance {
        Provenance::Declared(entries) => entries,
        Provenance::Unknown => return Vec::new(),
    };

    if declared_entries.is_empty() {
        return Vec::new();
    }

    // Build a map from output column alias -> set of bare column names actually read
    let mut alias_to_refs: HashMap<String, HashSet<String>> = HashMap::new();

    if let Some(select_list) = select.select_list() {
        for item in select_list.items() {
            let Some(col_name) = item.column_name() else {
                continue;
            };
            let expr_refs = if let Some(expr) = item.expression() {
                collect_expr_column_refs(expr.syntax())
            } else {
                Vec::new()
            };
            // Store only bare names (strip qualifier)
            let bare_names: HashSet<String> = expr_refs
                .into_iter()
                .map(|(_qualifier, name)| name)
                .collect();
            alias_to_refs.insert(col_name, bare_names);
        }
    }

    let mut out = Vec::new();

    for (output_col, source_cols) in declared_entries {
        // Normalize declared source cols: strip qualifier prefix to get bare name
        let declared_names: HashSet<String> = source_cols
            .iter()
            .map(|s| {
                // Strip qualifier: "source.revenue" -> "revenue"
                match s.rfind('.') {
                    Some(dot_pos) => s[dot_pos + 1..].to_string(),
                    None => s.clone(),
                }
            })
            .collect();

        let actual_names = match alias_to_refs.get(output_col) {
            Some(names) => names.clone(),
            None => {
                // Output column not found in SELECT items
                out.push(Diagnostic {
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "provenance declares output column `{output_col}` but it was not found \
                         in the SELECT list"
                    ),
                    range: anchor_range,
                    code: Some(DiagnosticCode::ProvenanceMismatch),
                    data: None,
                });
                continue;
            }
        };

        // Declared but not actually read
        for extra in declared_names.difference(&actual_names) {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "provenance for `{output_col}` declares source column `{extra}` but the \
                     body does not read it"
                ),
                range: anchor_range,
                code: Some(DiagnosticCode::ProvenanceMismatch),
                data: None,
            });
        }

        // Actually read but not declared
        for missing in actual_names.difference(&declared_names) {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "provenance for `{output_col}` does not declare source column `{missing}` \
                     but the body reads it"
                ),
                range: anchor_range,
                code: Some(DiagnosticCode::ProvenanceMismatch),
                data: None,
            });
        }
    }

    out
}

/// Validate declared `joins:` entries against the body's outermost FROM clause.
///
/// For each declared `JoinSpec`:
/// - Emits `DeclaredCardinalityUnverifiable` (Warning) if `cardinality` is non-empty
/// - Emits `JoinsMismatch` (Error) if the declared table name is not found as
///   a join alias (or identifier) in the body's FROM clause joins
pub fn validate_joins(
    select: &SelectStmt,
    joins: &[JoinSpec],
    anchor_range: rowan::TextRange,
) -> Vec<Diagnostic> {
    if joins.is_empty() {
        return Vec::new();
    }

    // Collect actual join names (alias or identifier) from the body's FROM clause
    let mut actual_join_names: HashSet<String> = HashSet::new();
    if let Some(from_clause) = select.from_clause() {
        for join in from_clause.joins() {
            if let Some(table_ref) = join.table_ref() {
                // Prefer alias; fall back to identifier
                let name = table_ref.alias().or_else(|| table_ref.identifier());
                if let Some(name) = name {
                    actual_join_names.insert(name);
                }
            }
        }
    }

    let mut out = Vec::new();

    for spec in joins {
        // Always warn about unverifiable cardinality when non-empty
        if !spec.cardinality.is_empty() {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Warning,
                message: format!(
                    "declared cardinality `{}` for join on `{}` cannot be verified statically \
                     (§20E soundness caveat — this is trusted, not checked against data)",
                    spec.cardinality, spec.table
                ),
                range: anchor_range,
                code: Some(DiagnosticCode::DeclaredCardinalityUnverifiable),
                data: None,
            });
        }

        // Error if declared table not found in body joins
        if !actual_join_names.contains(&spec.table) {
            out.push(Diagnostic {
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "declared join on `{}` not found in the body's FROM clause; \
                     actual join aliases: [{}]",
                    spec.table,
                    {
                        let mut names: Vec<_> = actual_join_names.iter().cloned().collect();
                        names.sort();
                        names.join(", ")
                    }
                ),
                range: anchor_range,
                code: Some(DiagnosticCode::JoinsMismatch),
                data: None,
            });
        }
    }

    out
}

/// Phase 51 — per-file provenance/joins validator.
///
/// Runs only when `unstable_schema: true` (caller must gate on this).
/// For each `smelt.define` in the file that declares `provenance:` or `joins:`
/// in its frontmatter, validates those declarations against the function body.
pub fn provenance_validator_diagnostics_for_file(
    db: &dyn salsa::Database,
    file: crate::SourceFile,
) -> Vec<Diagnostic> {
    use smelt_parser::File as AstFile;
    use smelt_planner::logical::Provenance;

    let raw_text = file.text(db);
    let parse = crate::parse_file(db, file);
    let syntax = parse.syntax();
    let Some(ast) = AstFile::cast(syntax) else {
        return Vec::new();
    };
    let sigs = crate::file_signature_inputs(db, file);
    let mut out = Vec::new();

    for define in ast.defines() {
        let Some(fm) = define.frontmatter(raw_text) else {
            continue;
        };
        let (props, _) = smelt_planner::logical::parse_function_properties(&fm);

        // Skip if no declared provenance or joins
        let has_provenance = matches!(props.provenance, Provenance::Declared(_));
        let has_joins = !props.joins.is_empty();
        if !has_provenance && !has_joins {
            continue;
        }

        // Get anchor range from cached signature list
        let name = define.name().unwrap_or_default();
        let anchor = sigs
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.name_range)
            .unwrap_or(rowan::TextRange::empty(rowan::TextSize::from(0)));

        // Get the outermost SELECT from the body
        let Some(body) = define.body() else {
            continue;
        };
        let Some(select) = body.select_stmt() else {
            continue;
        };

        if has_provenance {
            out.extend(validate_provenance(&select, &props.provenance, anchor));
        }
        if has_joins {
            out.extend(validate_joins(&select, &props.joins, anchor));
        }
    }

    out
}
