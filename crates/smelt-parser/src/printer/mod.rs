/// SQL printer for converting AST back to SQL
///
/// This module provides Display implementations for AST nodes to enable
/// round-trip testing (parse → print → parse).
///
/// Formatting rules:
/// - Keywords: UPPERCASE
/// - Identifiers: preserve case
/// - Indentation: 2 spaces (in Pretty mode)
/// - Line breaks: at major clauses (in Pretty mode)
use crate::ast::*;
use crate::SyntaxKind::*;

/// Format mode for SQL printing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatMode {
    /// Single-line output (no line breaks)
    Compact,
    /// Multi-line with indentation
    Pretty,
}

/// Context for formatting SQL
#[derive(Debug, Clone)]
#[allow(dead_code)] // Will be used for pretty printing in future
pub struct FormatContext {
    mode: FormatMode,
    indent_level: usize,
}

#[allow(dead_code)] // Will be used for pretty printing in future
impl FormatContext {
    pub fn new(mode: FormatMode) -> Self {
        Self {
            mode,
            indent_level: 0,
        }
    }

    pub fn compact() -> Self {
        Self::new(FormatMode::Compact)
    }

    pub fn pretty() -> Self {
        Self::new(FormatMode::Pretty)
    }

    fn indent(&self) -> String {
        if self.mode == FormatMode::Compact {
            String::new()
        } else {
            "  ".repeat(self.indent_level)
        }
    }

    fn newline(&self) -> &str {
        if self.mode == FormatMode::Compact {
            " "
        } else {
            "\n"
        }
    }

    fn with_indent(&self) -> Self {
        Self {
            mode: self.mode,
            indent_level: self.indent_level + 1,
        }
    }
}

mod clauses;
mod helpers;
#[cfg(test)]
mod tests;
