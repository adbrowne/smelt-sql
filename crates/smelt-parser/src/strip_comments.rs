//! Token-level comment stripping.
//!
//! `strip_sql_comments` removes `COMMENT` trivia tokens (both `--` line
//! comments and nesting-aware `/* */` block comments — see
//! `crate::lexer::Lexer::consume_comment`/`consume_block_comment`) from SQL
//! text while reproducing every other token's original text and the
//! whitespace between tokens exactly.
//!
//! This is a distinct code path from `crate::printer`, which regenerates
//! formatted SQL from the AST and drops comments only as a side effect of
//! reformatting everything. Using the printer to implement "remove
//! comments" would silently reformat the input; this function only removes
//! what was asked for. See `docs/specs/ui_model_diagnostics.md` §Design
//! "Why comment stripping is a new token-level utility, not the printer".

use crate::lexer::tokenize;
use crate::syntax_kind::SyntaxKind;

/// Return `sql` with every `COMMENT` token removed and all other text
/// (including whitespace layout) preserved byte-for-byte.
///
/// Operates purely on the lexer's token stream: it walks the tokens
/// `crate::lexer::tokenize` produces, reconstructing the original text by
/// concatenating each non-`COMMENT` token's source span and skipping
/// `COMMENT` spans entirely. Idempotent — a `COMMENT`-free token stream
/// stripped again is unchanged — because the lexer never re-tokenizes
/// removed comment text into a new comment.
pub fn strip_sql_comments(sql: &str) -> String {
    let tokens = tokenize(sql);
    let mut out = String::with_capacity(sql.len());
    let mut pos = 0usize;

    for token in tokens {
        let end = pos + token.len;
        if token.kind != SyntaxKind::COMMENT {
            out.push_str(&sql[pos..end]);
        }
        pos = end;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_line_comment() {
        let result = strip_sql_comments("SELECT 1 -- comment\n");
        assert_eq!(result, "SELECT 1 \n");
    }

    #[test]
    fn strips_block_comment() {
        let result = strip_sql_comments("SELECT /* c */ 1");
        assert_eq!(result, "SELECT  1");
    }

    #[test]
    fn no_comments_unchanged() {
        let input = "SELECT 1 FROM t";
        assert_eq!(strip_sql_comments(input), input);
    }
}
