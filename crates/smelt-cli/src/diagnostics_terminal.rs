//! Terminal diagnostic boundary converter.
//!
//! Converts `smelt_db::Diagnostic` range values to human-readable
//! `(line, column)` pairs for terminal output at the boundary between smelt's
//! analysis layer and the CLI's human-readable surface.

use line_index::LineIndex;

use smelt_db::Diagnostic;

/// Converts smelt diagnostics to terminal-friendly `(line, column)` pairs.
///
/// Constructed once per file at the CLI / analysis boundary. Holds a
/// `LineIndex` built from the file text for O(log N) per-diagnostic lookups.
///
/// Pure: takes no Salsa database reference; all inputs are plain data.
pub struct TerminalConverter {
    line_index: LineIndex,
}

impl TerminalConverter {
    /// Construct a converter for `text`.
    pub fn new(text: &str) -> Self {
        Self {
            line_index: LineIndex::new(text),
        }
    }

    /// Return `(line, column)` for the start of `diag`'s range, suitable for
    /// terminal output. Converts the byte-offset `TextRange` to a
    /// `(line, col)` pair via `LineIndex`.
    pub fn start(&self, diag: &Diagnostic) -> (u32, u32) {
        let lc = self.line_index.line_col(diag.range.start());
        (lc.line, lc.col)
    }
}
