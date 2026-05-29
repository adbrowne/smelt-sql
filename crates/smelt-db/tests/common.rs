//! Shared test helpers for `smelt-db` integration test crates.
//!
//! Each test file that needs position-asserting helpers adds:
//! ```
//! #[path = "common.rs"]
//! mod common;
//! ```
//! and calls `common::line_col_of(diag, text)` to convert a `TextRange`-based
//! diagnostic position to `(line, column)` for assertions.

#![allow(dead_code)]

use line_index::{LineIndex, TextSize};
use smelt_db::Diagnostic;

/// Return the `(line, column)` pair for the **start** of a diagnostic's range.
///
/// Uses `line_index::LineIndex` which counts columns in **bytes** — the same
/// unit stored in `Diagnostic::range`. For ASCII source text byte-column and
/// character-column coincide; this suffices for all existing smelt-db tests
/// (which use ASCII fixtures).
pub fn line_col_of(diag: &Diagnostic, text: &str) -> (u32, u32) {
    let li = LineIndex::new(text);
    let lc = li.line_col(TextSize::from(u32::from(diag.range.start())));
    (lc.line, lc.col)
}

/// Return the `(line, column)` pair for the **end** of a diagnostic's range.
pub fn end_line_col_of(diag: &Diagnostic, text: &str) -> (u32, u32) {
    let li = LineIndex::new(text);
    let lc = li.line_col(TextSize::from(u32::from(diag.range.end())));
    (lc.line, lc.col)
}

/// Return the byte length of a diagnostic's range (end - start).
pub fn range_len(diag: &Diagnostic) -> u32 {
    u32::from(diag.range.len())
}
