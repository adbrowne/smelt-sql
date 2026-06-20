//! Pure functions for generating code action suggestions from diagnostics.
//!
//! These functions are used by the LSP server and integration tests.
//! They follow the pure function rule: no Salsa/LSP dependencies.

use std::path::PathBuf;

use crate::{Diagnostic, DiagnosticCode, DiagnosticData};

/// A suggested code action with title and replacement text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeActionSuggestion {
    /// Human-readable title shown in the editor (e.g., "CAST as INTEGER")
    pub title: String,
    /// The replacement text for the diagnostic range
    pub new_text: String,
    /// The range to replace (byte-offset TextRange)
    pub range: rowan::TextRange,
}

/// A code action that creates a new model file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateModelSuggestion {
    /// Human-readable title (e.g., "Create model 'foo'")
    pub title: String,
    /// Name of the model to create (without .sql extension)
    pub model_name: String,
    /// Skeleton SQL content for the new file
    pub skeleton_sql: String,
}

/// A code action that inserts lines into a YAML file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YamlEditSuggestion {
    /// Human-readable title (e.g., "Add table 'users' to source 'raw'")
    pub title: String,
    /// 0-indexed line number after which to insert (use usize::MAX to append at end)
    pub insert_after_line: usize,
    /// Lines to insert (each line includes its own indentation)
    pub new_lines: Vec<String>,
}

/// A code action that creates a sidecar `.yml` next to a seed CSV file.
///
/// The `inferred_columns` list is in CSV header order. The LSP handler
/// serialises this into YAML and writes it as a new file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinSeedSchemaSuggestion {
    /// Human-readable title shown in the editor.
    pub title: String,
    /// Absolute path to the CSV file.
    pub csv_path: PathBuf,
    /// Absolute path where the sidecar `.yml` should be written.
    pub sidecar_path: PathBuf,
    /// Inferred columns: `(name, DataType)` pairs in CSV header order,
    /// derived by running the full-CSV inferencer over the source file.
    pub inferred_columns: Vec<(String, smelt_types::DataType)>,
}

/// All possible code action suggestions returned by the pure functions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeActionKind {
    /// Edit text within the current file (CAST, etc.)
    TextEdit(CodeActionSuggestion),
    /// Create a new model file
    CreateModel(CreateModelSuggestion),
    /// Edit the sources.yml file
    YamlEdit(YamlEditSuggestion),
    /// Create a sidecar `.yml` for a seed CSV that has no schema declaration.
    PinSeedSchema(PinSeedSchemaSuggestion),
}

/// Common SQL types offered when we can't infer the type.
const COMMON_TYPES: &[&str] = &[
    "VARCHAR",
    "INTEGER",
    "BIGINT",
    "DOUBLE",
    "BOOLEAN",
    "DATE",
    "TIMESTAMP",
];

/// Extract the text covered by a diagnostic range from the file text.
fn extract_range_text(file_text: &str, range: &rowan::TextRange) -> Option<String> {
    let start = usize::from(range.start());
    let end = usize::from(range.end());
    if start <= end && end <= file_text.len() {
        Some(file_text[start..end].to_string())
    } else {
        None
    }
}

/// Generate code action suggestions for a single diagnostic.
///
/// Returns an empty vec if no code actions apply.
pub fn generate_code_actions(
    diagnostic: &Diagnostic,
    file_text: &str,
) -> Vec<CodeActionSuggestion> {
    let code = match &diagnostic.code {
        Some(c) => c,
        None => return vec![],
    };

    match code {
        DiagnosticCode::TypeMismatch => generate_type_mismatch_actions(diagnostic, file_text),
        DiagnosticCode::CannotInferType | DiagnosticCode::ColumnTypeUnresolved => {
            generate_cannot_infer_actions(diagnostic, file_text)
        }
        _ => vec![],
    }
}

fn generate_type_mismatch_actions(
    diagnostic: &Diagnostic,
    file_text: &str,
) -> Vec<CodeActionSuggestion> {
    let expr_text = match extract_range_text(file_text, &diagnostic.range) {
        Some(t) => t,
        None => return vec![],
    };

    // Extract the expected type from DiagnosticData if available
    let target_type = match &diagnostic.data {
        Some(DiagnosticData::TypeMismatch { expected_type, .. }) => expected_type.clone(),
        _ => {
            // Fall back to parsing from message
            return vec![];
        }
    };

    vec![CodeActionSuggestion {
        title: format!("CAST as {}", target_type),
        new_text: format!("CAST({} AS {})", expr_text, target_type),
        range: diagnostic.range,
    }]
}

fn generate_cannot_infer_actions(
    diagnostic: &Diagnostic,
    file_text: &str,
) -> Vec<CodeActionSuggestion> {
    let expr_text = match extract_range_text(file_text, &diagnostic.range) {
        Some(t) => t,
        None => return vec![],
    };

    COMMON_TYPES
        .iter()
        .map(|ty| CodeActionSuggestion {
            title: format!("CAST as {}", ty),
            new_text: format!("CAST({} AS {})", expr_text, ty),
            range: diagnostic.range,
        })
        .collect()
}

/// Generate all code action kinds (text edits, create model, YAML edits) for a diagnostic.
///
/// This is the extended version of `generate_code_actions` that also handles
/// UndefinedModelRef, UndefinedSource, and UndeclaredColumn diagnostics.
pub fn generate_all_code_actions(
    diagnostic: &Diagnostic,
    file_text: &str,
    sources_yml: &str,
) -> Vec<CodeActionKind> {
    let code = match &diagnostic.code {
        Some(c) => c,
        None => return vec![],
    };

    match code {
        DiagnosticCode::TypeMismatch => generate_type_mismatch_actions(diagnostic, file_text)
            .into_iter()
            .map(CodeActionKind::TextEdit)
            .collect(),
        DiagnosticCode::CannotInferType => generate_cannot_infer_actions(diagnostic, file_text)
            .into_iter()
            .map(CodeActionKind::TextEdit)
            .collect(),
        DiagnosticCode::UndefinedModelRef => generate_create_model_action(diagnostic),
        DiagnosticCode::UndefinedSource => generate_add_source_action(diagnostic, sources_yml),
        DiagnosticCode::UndeclaredColumn => generate_add_column_action(diagnostic, sources_yml),
        DiagnosticCode::MissingSeedSidecar => generate_pin_seed_schema_action(diagnostic),
        _ => vec![],
    }
}

/// Generate a "Create model" code action for an undefined model reference.
fn generate_create_model_action(diagnostic: &Diagnostic) -> Vec<CodeActionKind> {
    let model_name = match &diagnostic.data {
        Some(DiagnosticData::UndefinedRef { model_name }) => model_name.clone(),
        _ => return vec![],
    };

    let skeleton = format!(
        "SELECT\n    -- TODO: implement {}\n    1 AS placeholder\n",
        model_name
    );

    vec![CodeActionKind::CreateModel(CreateModelSuggestion {
        title: format!("Create model '{}'", model_name),
        model_name,
        skeleton_sql: skeleton,
    })]
}

/// Generate "Add source/table" YAML edit for an undefined source reference.
fn generate_add_source_action(diagnostic: &Diagnostic, sources_yml: &str) -> Vec<CodeActionKind> {
    let (source_name, table_name) = match &diagnostic.data {
        Some(DiagnosticData::UndefinedSource {
            source_name,
            table_name,
        }) => (source_name.clone(), table_name.clone()),
        _ => return vec![],
    };

    // Check if the source section already exists in the YAML
    let source_key = format!("{}:", source_name);
    let mut source_found = false;
    let mut tables_line: Option<usize> = None;
    let mut in_source = false;
    let mut in_tables = false;
    // Track the last line belonging to the tables section
    let mut last_tables_content_line: Option<usize> = None;

    for (i, line) in sources_yml.lines().enumerate() {
        let trimmed = line.trim();

        // Detect the source section (e.g., "  raw:")
        if !trimmed.starts_with('-') && trimmed.starts_with(&source_key) {
            source_found = true;
            in_source = true;
            in_tables = false;
            continue;
        }

        if in_source {
            if trimmed == "tables:" {
                tables_line = Some(i);
                in_tables = true;
                continue;
            }

            // New source-level key resets (non-indented or different source)
            if !trimmed.is_empty()
                && !trimmed.starts_with('-')
                && !trimmed.starts_with('#')
                && !line.starts_with(' ')
            {
                in_source = false;
                in_tables = false;
                continue;
            }
        }

        if in_source && in_tables && !trimmed.is_empty() {
            last_tables_content_line = Some(i);
        }
    }

    if let (true, Some(tl)) = (source_found, tables_line) {
        // Source exists with tables section — insert new table after last content line
        let insert_after = last_tables_content_line.unwrap_or(tl);

        let new_lines = vec![
            format!("      {}:", table_name),
            "        columns:".to_string(),
            "          - name: id".to_string(),
        ];

        vec![CodeActionKind::YamlEdit(YamlEditSuggestion {
            title: format!("Add table '{}' to source '{}'", table_name, source_name),
            insert_after_line: insert_after,
            new_lines,
        })]
    } else {
        // Source doesn't exist — add full source block at the end
        let last_line = sources_yml.lines().count().saturating_sub(1);

        let new_lines = vec![
            format!("  {}:", source_name),
            "    tables:".to_string(),
            format!("      {}:", table_name),
            "        columns:".to_string(),
            "          - name: id".to_string(),
        ];

        vec![CodeActionKind::YamlEdit(YamlEditSuggestion {
            title: format!("Add source '{}' with table '{}'", source_name, table_name),
            insert_after_line: last_line,
            new_lines,
        })]
    }
}

/// Generate "Add column" YAML edit for an undeclared column on a source qualifier.
fn generate_add_column_action(diagnostic: &Diagnostic, sources_yml: &str) -> Vec<CodeActionKind> {
    let (qualifier, column_name) = match &diagnostic.data {
        Some(DiagnosticData::UndeclaredColumn {
            qualifier: Some(q),
            column_name,
        }) => (q.clone(), column_name.clone()),
        _ => return vec![],
    };

    // Find the table matching the qualifier in the YAML and locate its columns section
    let table_key = format!("{}:", qualifier);
    let mut in_tables = false;
    let mut in_target_table = false;
    let mut in_columns = false;
    let mut last_column_line: Option<usize> = None;
    let mut columns_line: Option<usize> = None;
    let mut found_table = false;

    for (i, line) in sources_yml.lines().enumerate() {
        let trimmed = line.trim();
        let indent = line.len() - line.trim_start().len();

        if trimmed == "tables:" {
            in_tables = true;
            in_target_table = false;
            in_columns = false;
            continue;
        }

        if in_tables && trimmed.starts_with(&table_key) && indent >= 4 {
            in_target_table = true;
            in_columns = false;
            found_table = true;
            continue;
        }

        // Reset on new table key at the same or lower indent
        if in_target_table
            && !trimmed.is_empty()
            && indent <= 6
            && trimmed.ends_with(':')
            && trimmed != "columns:"
        {
            if in_columns {
                // We already found columns, stop
                break;
            }
            in_target_table = false;
            in_columns = false;
            continue;
        }

        if in_target_table && trimmed == "columns:" {
            in_columns = true;
            columns_line = Some(i);
            continue;
        }

        if in_columns && trimmed.starts_with("- name:") {
            last_column_line = Some(i);
        }

        // Check for multi-line column entries (type:, description:, etc.)
        if in_columns
            && !trimmed.is_empty()
            && !trimmed.starts_with('-')
            && !trimmed.starts_with('#')
            && (trimmed.starts_with("type:") || trimmed.starts_with("description:"))
        {
            last_column_line = Some(i);
        }
    }

    if found_table {
        if let Some(cl) = columns_line {
            let insert_after = last_column_line.unwrap_or(cl);
            vec![CodeActionKind::YamlEdit(YamlEditSuggestion {
                title: format!(
                    "Add column '{}' to source table '{}'",
                    column_name, qualifier
                ),
                insert_after_line: insert_after,
                new_lines: vec![format!("          - name: {}", column_name)],
            })]
        } else {
            vec![]
        }
    } else {
        vec![]
    }
}

/// Generate a "Pin schema to sidecar YAML" code action for a
/// `MissingSeedSidecar` diagnostic.
///
/// Reads the CSV from disk, runs the full inferencer (all rows, no limit),
/// and returns a `PinSeedSchema` suggestion whose `inferred_columns` list
/// the LSP handler serialises into YAML.
fn generate_pin_seed_schema_action(diagnostic: &Diagnostic) -> Vec<CodeActionKind> {
    let (csv_path, sidecar_path) = match &diagnostic.data {
        Some(crate::DiagnosticData::MissingSeedSidecar {
            csv_path,
            sidecar_path,
        }) => (csv_path.clone(), sidecar_path.clone()),
        _ => return vec![],
    };

    // Read the CSV and infer types over all rows (no sample limit).
    let inferred_columns = match smelt_core::read_csv(&csv_path) {
        // intentionally ignored: if the CSV is unreadable we simply produce
        // no code-action suggestion — a silent empty list is correct here.
        Err(_) => return vec![],
        Ok((headers, rows_iter)) => {
            let rows: Vec<_> = rows_iter.filter_map(|r| r.ok()).collect();
            smelt_core::infer_columns(&rows, &headers, None)
        }
    };

    vec![CodeActionKind::PinSeedSchema(PinSeedSchemaSuggestion {
        title: "Pin schema to sidecar YAML".to_string(),
        csv_path,
        sidecar_path,
        inferred_columns,
    })]
}

/// A text edit: replace a range with new text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextEditSuggestion {
    pub range: rowan::TextRange,
    pub new_text: String,
}

/// Result of an "Extract CTE" refactoring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractCteResult {
    /// Human-readable title (e.g., "Extract to CTE 'sub'")
    pub title: String,
    /// The name chosen for the new CTE
    pub cte_name: String,
    /// The text edits to apply (ordered: CTE insertion first, then subquery replacement)
    pub edits: Vec<TextEditSuggestion>,
}

/// Result of an "Inline CTE" refactoring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineCteResult {
    /// Human-readable title (e.g., "Inline CTE 'sub'")
    pub title: String,
    /// The name of the CTE being inlined
    pub cte_name: String,
    /// The text edits to apply (ordered: reference replacement first, then CTE removal)
    pub edits: Vec<TextEditSuggestion>,
}

/// Find a CTE at the given cursor position that can be inlined, and generate the refactoring edits.
///
/// A CTE can be inlined if it is used exactly once in the query body.
/// The CTE's body replaces its single reference as a subquery, and the CTE definition is removed.
/// If the CTE is used 0 times, offers "Remove unused CTE" instead.
/// If the CTE is used >1 times, returns None (no action).
pub fn find_inline_cte_suggestion(file_text: &str, line: u32, col: u32) -> Option<InlineCteResult> {
    use rowan::TextSize;
    use smelt_parser::ast;
    use smelt_parser::syntax_kind::SyntaxKind;

    let parse = smelt_parser::parse(file_text);
    let root = parse.syntax();
    let offset = smelt_parser::symbol::position_to_offset(file_text, line, col);
    let offset = TextSize::from(offset as u32);

    // Find the SELECT statement and its WITH clause
    let select_stmt_node = root
        .children()
        .find(|n| n.kind() == SyntaxKind::SELECT_STMT)?;
    let select_stmt = ast::SelectStmt::cast(select_stmt_node.clone())?;
    let with_clause = select_stmt.with_clause()?;

    // Find the CTE at the cursor position
    let target_cte = with_clause
        .ctes()
        .find(|cte| cte.syntax().text_range().contains(offset))?;
    let cte_name = target_cte.name()?;

    // Get the CTE body text from the SUBQUERY node in the CTE syntax tree
    // In a CTE, the SUBQUERY node contains just the SELECT (no parens — parens are siblings)
    let subquery_node = target_cte
        .syntax()
        .children()
        .find(|n| n.kind() == SyntaxKind::SUBQUERY)?;
    let body = subquery_node.text().to_string().trim().to_string();

    // Count FROM/JOIN table references to this CTE (not qualifier uses like cte.col)
    let mut from_join_refs: Vec<rowan::TextRange> = Vec::new();

    // Find the main query's FROM clause (outside the WITH clause)
    if let Some(from_clause) = select_stmt.from_clause() {
        for table_ref in from_clause.table_refs() {
            if table_ref.function_call().is_some() || table_ref.subquery().is_some() {
                continue;
            }
            if table_ref.identifier().as_deref() == Some(cte_name.as_str()) {
                // Get the full TABLE_REF range (includes alias if any)
                from_join_refs.push(table_ref.syntax().text_range());
            }
        }
        for join in from_clause.joins() {
            if let Some(table_ref) = join.table_ref() {
                if table_ref.function_call().is_some() || table_ref.subquery().is_some() {
                    continue;
                }
                if table_ref.identifier().as_deref() == Some(cte_name.as_str()) {
                    from_join_refs.push(table_ref.syntax().text_range());
                }
            }
        }
    }

    if from_join_refs.len() > 1 {
        return None; // Too many FROM/JOIN references — can't inline safely
    }

    let mut edits = Vec::new();
    let all_ctes: Vec<_> = with_clause.ctes().collect();
    let is_only_cte = all_ctes.len() == 1;

    if from_join_refs.len() == 1 {
        // Replace the single FROM/JOIN reference with `(body) alias` as a subquery
        // Use the CTE name as the alias so qualifiers like `cte.col` still work
        let ref_range = from_join_refs[0];
        edits.push(TextEditSuggestion {
            range: ref_range,
            new_text: format!("({}) {}", body, cte_name),
        });
    }
    // For 0 usages we just remove the CTE (no reference replacement needed)

    // Remove the CTE from the WITH clause
    if is_only_cte {
        // Remove the entire WITH clause including trailing whitespace/newline
        let wc_range = with_clause.syntax().text_range();
        let wc_end = wc_range.end();
        // Consume whitespace between WITH clause and SELECT
        let mut end = wc_end;
        for token in select_stmt_node.children_with_tokens() {
            if let Some(t) = token.as_token() {
                if t.text_range().start() >= wc_end {
                    if t.kind() == SyntaxKind::WHITESPACE {
                        end = t.text_range().end();
                    }
                    break;
                }
            }
        }
        let removal_range = rowan::TextRange::new(wc_range.start(), end);
        edits.push(TextEditSuggestion {
            range: removal_range,
            new_text: String::new(),
        });
    } else {
        // Remove just this CTE from the WITH clause, keeping others
        let cte_removal_range = compute_cte_removal_range(&target_cte, &all_ctes);
        edits.push(TextEditSuggestion {
            range: cte_removal_range,
            new_text: String::new(),
        });
    }

    let title = if from_join_refs.is_empty() {
        format!("Remove unused CTE '{}'", cte_name)
    } else {
        format!("Inline CTE '{}'", cte_name)
    };

    Some(InlineCteResult {
        title,
        cte_name,
        edits,
    })
}

/// Compute the range to remove for a single CTE within a multi-CTE WITH clause.
/// Handles the comma separator (removes trailing comma if last CTE, leading comma if not).
fn compute_cte_removal_range(
    target_cte: &smelt_parser::ast::Cte,
    all_ctes: &[smelt_parser::ast::Cte],
) -> rowan::TextRange {
    use smelt_parser::syntax_kind::SyntaxKind;

    let target_range = target_cte.syntax().text_range();
    let is_last = all_ctes.last().map(|c| c.syntax().text_range()) == Some(target_range);

    if is_last {
        // Last CTE: remove the comma before it + whitespace + the CTE itself
        let parent = target_cte.syntax().parent().unwrap();
        let mut comma_start = target_range.start();
        let tokens_before: Vec<_> = parent
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| t.text_range().end() <= target_range.start())
            .collect();
        for t in tokens_before.iter().rev() {
            if t.kind() == SyntaxKind::COMMA {
                comma_start = t.text_range().start();
                break;
            } else if t.kind() == SyntaxKind::WHITESPACE {
                comma_start = t.text_range().start();
            } else {
                break;
            }
        }
        rowan::TextRange::new(comma_start, target_range.end())
    } else {
        // Not last CTE: remove the CTE + comma after it + whitespace
        let parent = target_cte.syntax().parent().unwrap();
        let mut end = target_range.end();
        let mut past_cte = false;
        for token in parent.children_with_tokens() {
            if let Some(t) = token.as_token() {
                if t.text_range().start() >= target_range.end() {
                    past_cte = true;
                }
                if past_cte {
                    if t.kind() == SyntaxKind::COMMA || t.kind() == SyntaxKind::WHITESPACE {
                        end = t.text_range().end();
                        continue;
                    } else {
                        break;
                    }
                }
            }
        }
        rowan::TextRange::new(target_range.start(), end)
    }
}

/// Find an extractable subquery at the given cursor position and generate the refactoring edits.
///
/// Returns `None` if the cursor is not inside a subquery within a FROM or JOIN clause.
pub fn find_extract_cte_suggestion(
    file_text: &str,
    line: u32,
    col: u32,
) -> Option<ExtractCteResult> {
    use rowan::TextSize;
    use smelt_parser::syntax_kind::SyntaxKind;

    let parse = smelt_parser::parse(file_text);
    let root = parse.syntax();
    let offset = smelt_parser::symbol::position_to_offset(file_text, line, col);
    let offset = TextSize::from(offset as u32);

    // Find the deepest SUBQUERY node at this offset
    let subquery_node = root
        .descendants()
        .filter(|n| n.kind() == SyntaxKind::SUBQUERY)
        .filter(|n| n.text_range().contains(offset))
        .last()?; // deepest (most specific)

    // Verify the subquery is inside a TABLE_REF (FROM or JOIN context)
    let table_ref_node = subquery_node
        .ancestors()
        .find(|n| n.kind() == SyntaxKind::TABLE_REF)?;

    // Verify the TABLE_REF is inside a FROM_CLAUSE or JOIN_CLAUSE
    let _from_or_join = table_ref_node
        .ancestors()
        .find(|n| n.kind() == SyntaxKind::FROM_CLAUSE || n.kind() == SyntaxKind::JOIN_CLAUSE)?;

    // Get the subquery body text (the content of the SUBQUERY node, including parens)
    let subquery_text = subquery_node.text().to_string();
    // The subquery node contains `(SELECT ...)` — extract just the SELECT body
    let body = subquery_text
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(&subquery_text)
        .trim();

    // Find the SELECT statement to check for existing WITH clause
    let select_stmt_node = root
        .children()
        .find(|n| n.kind() == SyntaxKind::SELECT_STMT)?;
    let select_stmt = smelt_parser::ast::SelectStmt::cast(select_stmt_node.clone())?;

    // Collect existing CTE names to generate a unique name
    let existing_ctes: Vec<String> = select_stmt
        .with_clause()
        .map(|wc| wc.ctes().filter_map(|c| c.name()).collect())
        .unwrap_or_default();

    let cte_name = generate_unique_cte_name(&existing_ctes);

    // Get the alias from the table_ref if present
    let table_ref = smelt_parser::ast::TableRef::cast(table_ref_node.clone())?;
    let alias = table_ref.alias();

    // The replacement for the TABLE_REF: just the CTE name (with original alias if different)
    let replacement = if let Some(ref a) = alias {
        if a == &cte_name {
            cte_name.clone()
        } else {
            format!("{} {}", cte_name, a)
        }
    } else {
        cte_name.clone()
    };

    let table_ref_range = table_ref_node.text_range();

    let mut edits = Vec::new();

    if let Some(with_clause) = select_stmt.with_clause() {
        // Append to existing WITH clause: insert ", cte_name AS (body)" after the last CTE
        let last_cte = with_clause.ctes().last()?;
        let last_cte_end = last_cte.syntax().text_range();
        // Insert after the last CTE
        edits.push(TextEditSuggestion {
            range: rowan::TextRange::empty(last_cte_end.end()),
            new_text: format!(",\n{} AS ({})", cte_name, body),
        });
    } else {
        // Create new WITH clause at the start of the SELECT statement
        let select_start = select_stmt_node.text_range();
        edits.push(TextEditSuggestion {
            range: rowan::TextRange::empty(select_start.start()),
            new_text: format!("WITH {} AS ({})\n", cte_name, body),
        });
    }

    // Replace the TABLE_REF (subquery + alias) with the CTE name
    edits.push(TextEditSuggestion {
        range: table_ref_range,
        new_text: replacement,
    });

    Some(ExtractCteResult {
        title: format!("Extract to CTE '{}'", cte_name),
        cte_name,
        edits,
    })
}

/// Generate a unique CTE name that doesn't conflict with existing ones.
fn generate_unique_cte_name(existing: &[String]) -> String {
    for i in 1.. {
        let name = format!("cte_{}", i);
        if !existing.iter().any(|n| n == &name) {
            return name;
        }
    }
    unreachable!()
}
