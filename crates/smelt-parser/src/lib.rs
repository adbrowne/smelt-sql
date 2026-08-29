pub mod ast;
pub mod lexer;
pub mod parser;
pub mod printer;
pub mod symbol;
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
pub use parser::{parse, parse_meta_expression_from_offset, Parse, ParseError};
pub use printer::{FormatContext, FormatMode};
pub use symbol::is_valid_sql_identifier;
pub use syntax_kind::SyntaxKind;

/// Re-export Rowan types for convenience
pub use rowan::{TextRange, TextSize};

/// A single `---` / `---` frontmatter block located in a source file.
///
/// Phase 11 of smelt-functions: files may contain multiple frontmatter
/// blocks, each attached to the declaration that immediately follows it
/// (`smelt.define` / `smelt.extern`). The legacy single-block-at-start
/// case is still supported for backwards compatibility with model files
/// (e.g. `examples/timeseries/models/daily_events.sql`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontmatterBlock {
    /// Byte range within the original source text covering the block,
    /// **inclusive** of both `---` delimiter lines and any trailing
    /// newline on the closing delimiter. Offsets point into the raw,
    /// unstripped source.
    pub range: std::ops::Range<usize>,
    /// The text between the opening and closing `---` delimiters
    /// (exclusive of the delimiters themselves). Line-ending style
    /// matches the source. An empty body is legal and produces `""`.
    pub inner_text: String,
}

/// Find every `---` / `---` frontmatter block in `text`.
///
/// A block is a pair of lines each containing only `---` (after
/// trimming). Blocks may appear anywhere in the file, not only at the
/// start — Phase 11 uses this to support per-declaration frontmatter.
///
/// The scanner is conservative: it only recognises lines whose
/// trimmed content is *exactly* `---`. Anything between an opening and
/// closing `---` is treated as the block body and returned verbatim in
/// [`FrontmatterBlock::inner_text`]. Nested `---` lines are not
/// supported — the first closing delimiter after an opening delimiter
/// terminates the block.
pub fn find_frontmatter_blocks(text: &str) -> Vec<FrontmatterBlock> {
    let mut blocks = Vec::new();
    let mut i = 0;
    let bytes = text.as_bytes();
    let len = bytes.len();

    // Walk line by line, tracking byte offsets.
    let mut line_start = 0usize;
    while line_start <= len {
        // Find the end of this line (exclusive of newline).
        let mut line_end = line_start;
        while line_end < len && bytes[line_end] != b'\n' {
            line_end += 1;
        }
        // End offset including the trailing newline (if any).
        let after_line = if line_end < len {
            line_end + 1
        } else {
            line_end
        };

        let line = &text[line_start..line_end];
        if line.trim() == "---" {
            // Opening delimiter. Hunt for a matching closing delimiter.
            let open_start = line_start;
            let open_end = after_line;

            let mut search_start = after_line;
            let mut found_close: Option<(usize, usize)> = None;
            while search_start <= len {
                let mut cand_end = search_start;
                while cand_end < len && bytes[cand_end] != b'\n' {
                    cand_end += 1;
                }
                let cand_after = if cand_end < len {
                    cand_end + 1
                } else {
                    cand_end
                };
                let cand_line = &text[search_start..cand_end];
                if cand_line.trim() == "---" {
                    found_close = Some((search_start, cand_after));
                    break;
                }
                if search_start >= len {
                    break;
                }
                if cand_end == len {
                    // Reached EOF mid-scan — bail.
                    break;
                }
                search_start = cand_after;
            }

            if let Some((close_start, close_end)) = found_close {
                // Inner text: everything between the line after the opening
                // delimiter and the start of the closing delimiter line.
                let inner_text = text[open_end..close_start].to_string();
                blocks.push(FrontmatterBlock {
                    range: open_start..close_end,
                    inner_text,
                });
                // Resume scanning after the closing delimiter.
                i = close_end;
                line_start = close_end;
                if line_start >= len {
                    break;
                }
                continue;
            } else {
                // Unterminated `---` opener. Treat it as not-a-block and
                // move past it.
                line_start = after_line;
                if line_start >= len {
                    break;
                }
                continue;
            }
        }

        if line_end == len {
            break;
        }
        line_start = after_line;
        let _ = i;
    }

    blocks
}

/// Replace every frontmatter block in `text` with whitespace-preserving
/// SQL comment lines.  This lets the parser see only valid SQL while
/// preserving line numbers for accurate diagnostics.
///
/// Phase 11: previously this only stripped a single block at the start
/// of the file. Now it strips every block returned by
/// [`find_frontmatter_blocks`] — the comment-replacement trick is
/// applied per line within each block so byte offsets remain stable.
/// Single-block files are a strict subset of this behaviour.
pub fn strip_frontmatter(text: &str) -> String {
    let blocks = find_frontmatter_blocks(text);
    if blocks.is_empty() {
        return text.to_string();
    }

    // Build a mask marking which byte offsets fall inside a frontmatter
    // block. We need line-level awareness during replacement so we keep
    // a sorted list of ranges instead of a single boolean per byte.
    let mut out = String::with_capacity(text.len());
    let mut idx = 0usize;
    let bytes = text.as_bytes();
    let len = bytes.len();

    // Helper: is a given (byte) offset inside any block?
    let in_block = |off: usize| {
        blocks
            .iter()
            .any(|b| off >= b.range.start && off < b.range.end)
    };

    while idx <= len {
        // Walk one line at a time. This lets us reproduce the exact
        // per-line comment-replacement used by the legacy strip.
        let mut line_end = idx;
        while line_end < len && bytes[line_end] != b'\n' {
            line_end += 1;
        }
        let has_newline = line_end < len;
        let line = &text[idx..line_end];

        if in_block(idx) {
            // Replace the line with `--` + spaces of matching length.
            // `line.len()` is byte length; frontmatter is ASCII in
            // practice so the resulting SQL comment has the same byte
            // length as the original line.
            let replacement = format!("--{}", " ".repeat(line.len().saturating_sub(2)));
            out.push_str(&replacement);
        } else {
            out.push_str(line);
        }

        if has_newline {
            out.push('\n');
            idx = line_end + 1;
        } else {
            break;
        }
    }

    out
}

/// For each declaration in `decls` (given as byte ranges of the
/// declaration-start in the raw source), find the frontmatter block
/// that immediately precedes it — i.e. the last block in the file whose
/// end offset lies before the declaration's start and which is NOT
/// separated from the declaration by another declaration.
///
/// Returns a `Vec` parallel to `decls`: `None` when no block is
/// attached, `Some((block_range, inner_text))` when one is.
pub fn attach_frontmatter_to_decls(text: &str, decls: &[usize]) -> Vec<Option<FrontmatterBlock>> {
    let blocks = find_frontmatter_blocks(text);
    let mut sorted_decls: Vec<(usize, usize)> = decls.iter().copied().enumerate().collect();
    sorted_decls.sort_by_key(|(_, start)| *start);

    let mut attached: Vec<Option<FrontmatterBlock>> = vec![None; decls.len()];

    for block in &blocks {
        // Find the first declaration whose start is >= block.range.end.
        let next = sorted_decls
            .iter()
            .find(|(_, start)| *start >= block.range.end);
        if let Some((decl_idx, decl_start)) = next {
            // Between block.range.end and decl_start there must be no
            // other decl. That's guaranteed by our "first" lookup.
            // But we also reject if the gap contains any non-whitespace
            // non-comment content — a block only attaches to the
            // *immediately* following decl.
            let gap = &text[block.range.end..*decl_start];
            if gap_is_whitespace_or_comments(gap) {
                attached[*decl_idx] = Some(block.clone());
            }
        }
    }

    attached
}

/// True when `s` contains only whitespace and SQL line-comments.
///
/// Used by [`attach_frontmatter_to_decls`] to decide whether a block
/// "immediately" precedes a declaration. A `---` block followed by
/// blank lines or comments still attaches; one interrupted by SQL or
/// another declaration does not.
fn gap_is_whitespace_or_comments(s: &str) -> bool {
    for line in s.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t.starts_with("--") {
            continue;
        }
        return false;
    }
    true
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

    #[test]
    fn test_find_frontmatter_blocks_single() {
        let input = "---\nfoo: bar\n---\nSELECT 1\n";
        let blocks = find_frontmatter_blocks(input);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].inner_text, "foo: bar\n");
    }

    #[test]
    fn test_find_frontmatter_blocks_multiple() {
        let input = "---\nbackends: [duckdb]\n---\nsmelt.define f() -> Expr<Integer> AS (1)\n\n---\nbackends: [spark]\n---\nsmelt.define g() -> Expr<Integer> AS (2)\n";
        let blocks = find_frontmatter_blocks(input);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].inner_text.contains("duckdb"));
        assert!(blocks[1].inner_text.contains("spark"));
        // Blocks should appear in source order.
        assert!(blocks[0].range.end <= blocks[1].range.start);
    }

    #[test]
    fn test_strip_frontmatter_preserves_line_numbers_multi_block() {
        let input = "---\nbackends: [duckdb]\n---\nsmelt.define f() -> Expr<Integer> AS (1)\n\n---\nbackends: [spark]\n---\nsmelt.define g() -> Expr<Integer> AS (2)\n";
        let stripped = strip_frontmatter(input);
        // Same line count, same byte length.
        assert_eq!(input.lines().count(), stripped.lines().count());
        assert_eq!(input.len(), stripped.len());
        // Frontmatter bodies are no longer present.
        assert!(!stripped.contains("backends: [duckdb]"));
        assert!(!stripped.contains("backends: [spark]"));
        // Both defines survived intact.
        assert!(stripped.contains("smelt.define f()"));
        assert!(stripped.contains("smelt.define g()"));
    }

    #[test]
    fn test_attach_frontmatter_to_decls() {
        let input = "---\nbackends: [duckdb]\n---\nsmelt.define f() -> Expr<Integer> AS (1)\n\n---\nbackends: [spark]\n---\nsmelt.define g() -> Expr<Integer> AS (2)\n";
        // Declarations start at `s` in "smelt.define f" and "smelt.define g".
        let f_start = input.find("smelt.define f").unwrap();
        let g_start = input.find("smelt.define g").unwrap();
        let attached = attach_frontmatter_to_decls(input, &[f_start, g_start]);
        assert_eq!(attached.len(), 2);
        assert!(attached[0].as_ref().unwrap().inner_text.contains("duckdb"));
        assert!(attached[1].as_ref().unwrap().inner_text.contains("spark"));
    }

    #[test]
    fn test_attach_frontmatter_only_attaches_to_immediate_decl() {
        // A `---` block separated from the decl by SQL should not attach.
        let input =
            "---\nbackends: [duckdb]\n---\nSELECT 1;\nsmelt.define f() -> Expr<Integer> AS (1)\n";
        let f_start = input.find("smelt.define f").unwrap();
        let attached = attach_frontmatter_to_decls(input, &[f_start]);
        assert!(attached[0].is_none());
    }
}
