//! LSP diagnostic boundary converter.
//!
//! Converts `smelt_db::Diagnostic` range values to `lsp_types::Range` at the
//! boundary between smelt's analysis layer and the LSP protocol. A
//! `BoundaryConverter` is constructed once per file from the file's text and
//! performs O(log N) lookups via `line_index::LineIndex`.

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
    /// `Diagnostic::range` is a `rowan::TextRange` (byte-offset form).
    /// This method converts both endpoints to wide (UTF-16) line/column
    /// positions via `LineIndex`.
    pub fn convert(&self, diag: &Diagnostic) -> Range {
        self.text_range_to_lsp(diag.range)
    }

    /// Convert any `rowan::TextRange` to an `lsp_types::Range`.
    pub fn text_range_to_lsp(&self, range: rowan::TextRange) -> Range {
        let start = self
            .to_wide_line_col(range.start())
            .map(|wlc| Position {
                line: wlc.line,
                character: wlc.col,
            })
            .unwrap_or(Position {
                line: 0,
                character: 0,
            });
        let end = self
            .to_wide_line_col(range.end())
            .map(|wlc| Position {
                line: wlc.line,
                character: wlc.col,
            })
            .unwrap_or(Position {
                line: 0,
                character: 0,
            });
        Range { start, end }
    }

    /// Return the `WideLineCol` for a `TextSize` (UTF-16 column units).
    pub fn to_wide_line_col(&self, ts: TextSize) -> Option<WideLineCol> {
        let lc = self.line_index.line_col(ts);
        self.line_index.to_wide(self.encoding, lc)
    }
}
