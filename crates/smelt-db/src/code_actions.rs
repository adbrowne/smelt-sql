//! Pure functions for generating code action suggestions from diagnostics.
//!
//! These functions are used by the LSP server and integration tests.
//! They follow the pure function rule: no Salsa/LSP dependencies.

use crate::{Diagnostic, DiagnosticCode, DiagnosticData};

/// A suggested code action with title and replacement text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeActionSuggestion {
    /// Human-readable title shown in the editor (e.g., "CAST as INTEGER")
    pub title: String,
    /// The replacement text for the diagnostic range
    pub new_text: String,
    /// The range to replace (line/col start and end)
    pub range: crate::Range,
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

/// All possible code action suggestions returned by the pure functions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeActionKind {
    /// Edit text within the current file (CAST, etc.)
    TextEdit(CodeActionSuggestion),
    /// Create a new model file
    CreateModel(CreateModelSuggestion),
    /// Edit the sources.yml file
    YamlEdit(YamlEditSuggestion),
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
fn extract_range_text(file_text: &str, range: &crate::Range) -> Option<String> {
    let mut current_line = 0u32;
    let mut current_col = 0u32;
    let mut start_byte = None;
    let mut end_byte = None;

    for (i, ch) in file_text.char_indices() {
        if current_line == range.start.line && current_col == range.start.column {
            start_byte = Some(i);
        }
        if current_line == range.end.line && current_col == range.end.column {
            end_byte = Some(i);
            break;
        }
        if ch == '\n' {
            current_line += 1;
            current_col = 0;
        } else {
            current_col += 1;
        }
    }

    // Handle end at EOF
    if end_byte.is_none() && current_line == range.end.line && current_col == range.end.column {
        end_byte = Some(file_text.len());
    }

    match (start_byte, end_byte) {
        (Some(s), Some(e)) => Some(file_text[s..e].to_string()),
        _ => None,
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
        DiagnosticCode::CannotInferType => generate_cannot_infer_actions(diagnostic, file_text),
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
