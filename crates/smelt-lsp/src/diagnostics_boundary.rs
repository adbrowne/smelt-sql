//! LSP diagnostic boundary converter.
//!
//! Converts `smelt_db::Diagnostic` range values to `lsp_types::Range` at the
//! boundary between smelt's analysis layer and the LSP protocol. A
//! `BoundaryConverter` is constructed once per file from the file's text and
//! performs O(log N) lookups via `line_index::LineIndex`.
//!
//! Shadow-mode: the `convert` call runs a `debug_assert` that `LineIndex`
//! agrees with the stored `(line, col)` values, so a future flip of
//! `Diagnostic::range` to `TextRange` will surface any divergence in dev/test
//! builds. In release builds the assertion compiles away.

use line_index::{LineIndex, TextSize, WideEncoding, WideLineCol};
use lsp_types::{Position, Range};

use smelt_db::Diagnostic;

/// Converts smelt diagnostics to LSP-protocol ranges.
///
/// Constructed once per file at the LSP / analysis boundary. Holds a
/// `LineIndex` built from the file text so each `convert` call is O(log N)
/// rather than O(N).
///
/// Pure: takes no Salsa database reference; all inputs are plain data.
pub struct BoundaryConverter {
    line_index: LineIndex,
    #[allow(dead_code)] // encoding negotiation wired when Diagnostic::range becomes TextRange
    encoding: WideEncoding,
}

impl BoundaryConverter {
    /// Construct a converter for `text`. `encoding` selects the column unit
    /// the LSP client requested; defaults to `WideEncoding::Utf16` (LSP
    /// default when the client advertises no preference).
    pub fn new(text: &str, encoding: WideEncoding) -> Self {
        Self {
            line_index: LineIndex::new(text),
            encoding,
        }
    }

    /// Construct a converter with the default LSP encoding (UTF-16).
    pub fn new_utf16(text: &str) -> Self {
        Self::new(text, WideEncoding::Utf16)
    }

    /// Convert a `Diagnostic`'s `range` field to an `lsp_types::Range`.
    ///
    /// While `Diagnostic::range` is `(line, column)`-shaped, this method
    /// round-trips through `LineIndex` and asserts agreement under
    /// `debug_assertions`. Once `Diagnostic::range` becomes
    /// `TextRange`-shaped, the round-trip is dropped and the conversion runs
    /// forward only.
    pub fn convert(&self, diag: &Diagnostic, text: &str) -> Range {
        #[cfg(debug_assertions)]
        self.shadow_assert(diag, text);

        // Pass-through: the Diagnostic::range is already (line, column).
        // When Diagnostic::range becomes TextRange-shaped this becomes a
        // TextSize → LineIndex lookup.
        Range {
            start: Position {
                line: diag.range.start.line,
                character: diag.range.start.column,
            },
            end: Position {
                line: diag.range.end.line,
                character: diag.range.end.column,
            },
        }
    }

    /// Shadow assert: recompute the `(line, column)` pair from the byte offset
    /// via `LineIndex` and assert it matches the stored value. Fires only
    /// under `debug_assertions`; no-op in release builds.
    #[cfg(debug_assertions)]
    fn shadow_assert(&self, diag: &Diagnostic, text: &str) {
        for (label, pos) in [("start", diag.range.start), ("end", diag.range.end)] {
            let Some(byte_off) = codepoint_position_to_byte_offset(text, pos.line, pos.column)
            else {
                // Position at or beyond EOF — skip the assert rather than
                // spuriously failing on trailing-position diagnostics.
                continue;
            };
            let lc = self.line_index.line_col(TextSize::from(byte_off as u32));
            debug_assert_eq!(
                lc.line, pos.line,
                "LineIndex line mismatch at diagnostic {label}: \
                 stored={}, derived={}; diag message={:?}",
                pos.line, lc.line, diag.message
            );
            debug_assert_eq!(
                lc.col, pos.column,
                "LineIndex col mismatch at diagnostic {label}: \
                 stored={}, derived={}; diag message={:?}",
                pos.column, lc.col, diag.message
            );
        }
    }

    /// Return the `WideLineCol` for a `TextSize` (UTF-16 column units).
    /// Used when `Diagnostic::range` becomes `TextRange`-shaped and the
    /// converter performs encoding-aware conversion.
    #[allow(dead_code)]
    pub fn to_wide_line_col(&self, ts: TextSize) -> Option<WideLineCol> {
        let lc = self.line_index.line_col(ts);
        self.line_index.to_wide(self.encoding, lc)
    }
}

/// Scan `text` to find the byte offset of the character at `(line, col_codepoints)`.
/// Returns `None` if the position is out of range.
///
/// `col_codepoints` is a Unicode codepoint index within the line, which is
/// how `smelt_parser::offset_to_position` stores the column today.
fn codepoint_position_to_byte_offset(text: &str, line: u32, col: u32) -> Option<usize> {
    let mut cur_line = 0u32;
    let mut cur_col = 0u32;
    for (i, ch) in text.char_indices() {
        if cur_line == line && cur_col == col {
            return Some(i);
        }
        if ch == '\n' {
            cur_line += 1;
            cur_col = 0;
        } else {
            cur_col += 1;
        }
    }
    if cur_line == line && cur_col == col {
        return Some(text.len());
    }
    None
}
