pub mod ast;
pub mod lexer;
pub mod parser;
pub mod printer;
/// smelt-parser - Rowan-based parser for smelt SQL files
///
/// This crate provides a standalone parser for smelt model files, which are
/// SQL files with template expressions like {{ ref('model_name') }}.
///
/// The parser is built on Rowan, providing:
/// - Lossless concrete syntax tree (CST)
/// - Error recovery (parse incomplete/invalid code)
/// - Position tracking for diagnostics and IDE features
///
/// This crate is standalone and can be used independently of the LSP or Salsa.
pub mod syntax_kind;

pub use ast::*;
pub use parser::{parse, Parse, ParseError};
pub use printer::{FormatContext, FormatMode};
pub use syntax_kind::SyntaxKind;

/// Re-export Rowan types for convenience
pub use rowan::TextRange;

/// Replace YAML frontmatter (`---` delimited block at start of file) with
/// SQL comment lines of matching length.  This lets the parser see only valid
/// SQL while preserving line numbers for accurate diagnostics.
pub fn strip_frontmatter(text: &str) -> String {
    let trimmed = text.trim_start();
    if !trimmed.starts_with("---\n") && !trimmed.starts_with("---\r\n") {
        return text.to_string();
    }

    let lines: Vec<&str> = text.lines().collect();
    // Find the opening ---
    let open = match lines.iter().position(|l| l.trim() == "---") {
        Some(i) => i,
        None => return text.to_string(),
    };
    // Find the closing ---
    let close = match lines.iter().skip(open + 1).position(|l| l.trim() == "---") {
        Some(i) => open + 1 + i,
        None => return text.to_string(),
    };

    let mut result = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        if i >= open && i <= close {
            result.push(format!("--{}", " ".repeat(line.len().saturating_sub(2))));
        } else {
            result.push(line.to_string());
        }
    }

    let mut joined = result.join("\n");
    if text.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_frontmatter_basic() {
        let input = "---\ntags:\n  - event_source\n---\nSELECT 1\n";
        let result = strip_frontmatter(input);
        assert!(result.contains("SELECT 1"));
        assert!(!result.contains("tags:"));
        assert!(!result.contains("event_source"));
        // Line count preserved
        assert_eq!(input.lines().count(), result.lines().count());
    }

    #[test]
    fn test_strip_frontmatter_no_frontmatter() {
        let input = "SELECT 1\nFROM foo\n";
        let result = strip_frontmatter(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_strip_frontmatter_comment_only() {
        let input = "-- just a comment\nSELECT 1\n";
        let result = strip_frontmatter(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_strip_frontmatter_preserves_line_numbers() {
        let input = "---\nname: my_model\ntags:\n  - core\n---\nSELECT 1\n";
        let result = strip_frontmatter(input);
        let result_lines: Vec<&str> = result.lines().collect();
        // SQL should still be on line 6 (0-indexed: 5)
        assert_eq!(result_lines[5], "SELECT 1");
    }

    #[test]
    fn test_strip_frontmatter_no_parse_errors() {
        let input = "---\ntags:\n  - event_source\n---\nSELECT event_id, user_id FROM foo\n";
        let clean = strip_frontmatter(input);
        let parsed = parse(&clean);
        assert!(
            parsed.errors.is_empty(),
            "Frontmatter should not cause parse errors, got: {:?}",
            parsed.errors
        );
    }
}
