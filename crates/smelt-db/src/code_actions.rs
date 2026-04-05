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
