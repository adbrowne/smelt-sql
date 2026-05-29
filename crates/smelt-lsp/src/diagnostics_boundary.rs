//! LSP diagnostic boundary converter.
//!
//! Converts `smelt_db::Diagnostic` range values to `lsp_types::Range` at the
//! boundary between smelt's analysis layer and the LSP protocol. A
//! `BoundaryConverter` is constructed once per file from the file's text and
//! performs O(log N) lookups via `line_index::LineIndex`.

use line_index::{LineIndex, TextSize, WideEncoding, WideLineCol};
use lsp_types::{Position, PositionEncodingKind, Range};

use smelt_db::Diagnostic;

/// Column encoding used when converting byte-offset positions to LSP positions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnEncoding {
    /// UTF-16 code units (LSP default when client advertises no preference).
    Utf16,
    /// UTF-8 bytes. `line_index::LineIndex::line_col` returns the byte offset
    /// on the line directly, so no wide conversion is needed.
    Utf8,
}

impl ColumnEncoding {
    /// Convert an `lsp_types::PositionEncodingKind` to a `ColumnEncoding`.
    /// Unrecognised kinds fall back to `Utf16` (LSP default).
    pub fn from_lsp_kind(kind: &PositionEncodingKind) -> Self {
        if *kind == PositionEncodingKind::UTF8 {
            Self::Utf8
        } else {
            // UTF-16 is both the LSP default and the fallback for any
            // encoding the server does not support (e.g. UTF-32).
            Self::Utf16
        }
    }
}

/// Converts smelt diagnostics to LSP-protocol ranges.
///
/// Constructed once per file at the LSP / analysis boundary. Holds a
/// `LineIndex` built from the file text so each `convert` call is O(log N)
/// rather than O(N).
///
/// Pure: takes no Salsa database reference; all inputs are plain data.
pub struct BoundaryConverter {
    line_index: LineIndex,
    encoding: ColumnEncoding,
}

impl BoundaryConverter {
    /// Construct a converter for `text` with the given column encoding.
    pub fn new_with_encoding(text: &str, encoding: ColumnEncoding) -> Self {
        Self {
            line_index: LineIndex::new(text),
            encoding,
        }
    }

    /// Construct a converter from a negotiated `PositionEncodingKind`.
    pub fn new_from_kind(text: &str, kind: &PositionEncodingKind) -> Self {
        Self::new_with_encoding(text, ColumnEncoding::from_lsp_kind(kind))
    }

    /// Construct a converter with the default LSP encoding (UTF-16).
    pub fn new_utf16(text: &str) -> Self {
        Self::new_with_encoding(text, ColumnEncoding::Utf16)
    }

    /// Construct a converter with UTF-8 byte-column encoding.
    pub fn new_utf8(text: &str) -> Self {
        Self::new_with_encoding(text, ColumnEncoding::Utf8)
    }

    // `new(text, WideEncoding)` is no longer the primary API; callers should
    // use `new_utf16`, `new_utf8`, or `new_from_kind`. This shim keeps any
    // remaining callers that pass a `WideEncoding` working.
    pub fn new(text: &str, wide: WideEncoding) -> Self {
        let encoding = match wide {
            WideEncoding::Utf16 => ColumnEncoding::Utf16,
            // Any other WideEncoding variant (e.g. Utf32) falls back to Utf16.
            _ => ColumnEncoding::Utf16,
        };
        Self::new_with_encoding(text, encoding)
    }

    /// Convert a `Diagnostic`'s `range` field to an `lsp_types::Range`.
    ///
    /// `Diagnostic::range` is a `rowan::TextRange` (byte-offset form).
    /// This method converts both endpoints to line/column positions in the
    /// negotiated encoding via `LineIndex`.
    pub fn convert(&self, diag: &Diagnostic) -> Range {
        self.text_range_to_lsp(diag.range)
    }

    /// Convert any `rowan::TextRange` to an `lsp_types::Range`.
    pub fn text_range_to_lsp(&self, range: rowan::TextRange) -> Range {
        let start = self.ts_to_position(range.start());
        let end = self.ts_to_position(range.end());
        Range { start, end }
    }

    /// Convert a `TextSize` (byte offset) to an `lsp_types::Position` in the
    /// negotiated encoding.
    fn ts_to_position(&self, ts: TextSize) -> Position {
        let lc = self.line_index.line_col(ts);
        match self.encoding {
            ColumnEncoding::Utf8 => {
                // `lc.col` is the byte offset on the line — exactly what UTF-8
                // clients expect.
                Position {
                    line: lc.line,
                    character: lc.col,
                }
            }
            ColumnEncoding::Utf16 => {
                // Convert byte-column to UTF-16 code-unit column.
                self.line_index
                    .to_wide(WideEncoding::Utf16, lc)
                    .map(|wlc| Position {
                        line: wlc.line,
                        character: wlc.col,
                    })
                    .unwrap_or(Position {
                        line: lc.line,
                        character: lc.col,
                    })
            }
        }
    }

    /// Return the `WideLineCol` for a `TextSize` (UTF-16 column units).
    ///
    /// Kept for callers that explicitly need the `WideLineCol` form (e.g.
    /// goto-definition range conversion in the non-diagnostic path).
    pub fn to_wide_line_col(&self, ts: TextSize) -> Option<WideLineCol> {
        let lc = self.line_index.line_col(ts);
        self.line_index.to_wide(WideEncoding::Utf16, lc)
    }
}
