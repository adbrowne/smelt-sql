//! Terminal diagnostic boundary converter.
//!
//! Converts `smelt_db::Diagnostic` range values to human-readable
//! `(line, column)` pairs for terminal output at the boundary between smelt's
//! analysis layer and the CLI's human-readable surface.
//!
//! Shadow-mode: the `start` call runs a `debug_assert` that `LineIndex` agrees
//! with the stored `(line, col)` values for ASCII workspaces. In release builds
//! the assertion compiles away. Once `Diagnostic::range` becomes
//! `TextRange`-shaped the pass-through is replaced with a codepoint-counting
//! path consistent with rustc terminal output.

use line_index::{LineIndex, TextSize};

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
    /// terminal output. Currently returns the values stored in
    /// `diag.range.start` directly; a shadow `debug_assert` verifies `LineIndex`
    /// agrees. Once `Diagnostic::range` becomes `TextRange`-shaped this becomes
    /// a codepoint-based path consistent with rustc terminal output.
    pub fn start(&self, diag: &Diagnostic, text: &str) -> (u32, u32) {
        #[cfg(debug_assertions)]
        self.shadow_assert(diag, text);

        (diag.range.start.line, diag.range.start.column)
    }

    /// Shadow assert: recompute via `LineIndex` and verify against stored values.
    /// Fires only under `debug_assertions`.
    #[cfg(debug_assertions)]
    fn shadow_assert(&self, diag: &Diagnostic, text: &str) {
        for (label, pos) in [("start", diag.range.start), ("end", diag.range.end)] {
            let Some(byte_off) = codepoint_position_to_byte_offset(text, pos.line, pos.column)
            else {
                continue;
            };
            let lc = self.line_index.line_col(TextSize::from(byte_off as u32));
            debug_assert_eq!(
                lc.line, pos.line,
                "TerminalConverter: LineIndex line mismatch at {label}: \
                 stored={}, derived={}; diag={:?}",
                pos.line, lc.line, diag.message
            );
            debug_assert_eq!(
                lc.col, pos.column,
                "TerminalConverter: LineIndex col mismatch at {label}: \
                 stored={}, derived={}; diag={:?}",
                pos.column, lc.col, diag.message
            );
        }
    }
}

/// Scan `text` to find the byte offset of the character at `(line, col_codepoints)`.
/// Returns `None` if the position is out of range.
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
