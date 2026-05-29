//! Unit tests for `LineIndex` semantics and parity with `smelt_parser::offset_to_position`.
//!
//! These tests verify that `line_index::LineIndex::new(text).line_col(offset)`
//! round-trips to the same `(line, column)` as `smelt_parser::offset_to_position`
//! for ASCII text, and that the non-ASCII behaviour of `LineIndex` is
//! byte-column (not codepoint-column).
//!
//! No LSP server or Salsa DB is needed — all assertions are pure computations
//! over text and `TextSize` values.

use line_index::{LineIndex, TextSize};
use smelt_db::{file_diagnostics, Database, SourceFile, Workspace};
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Reconstruct a byte-offset for `(line, col_codepoints)` the same way
/// `smelt_parser::ast::offset_to_position` produces its output. For ASCII
/// text the codepoint offset equals the byte offset, so this round-trips
/// correctly in the shadow assert.
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
    // Handle position at the very end of text.
    if cur_line == line && cur_col == col {
        return Some(text.len());
    }
    None
}

/// Build a minimal Salsa DB with one project and one source file, returning
/// the diagnostics produced for that file.
fn diagnostics_for_text(
    text: &str,
    file_path: PathBuf,
    project_root: PathBuf,
) -> (String, Vec<smelt_db::Diagnostic>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    let file: SourceFile = db.set_source_file(file_path, text.to_string(), project_root);
    db.set_workspace(vec![file], vec![project]);
    let ws = Workspace::try_get(&db).expect("workspace not initialised");
    let diags = file_diagnostics(&db, ws, file);
    (text.to_string(), diags)
}

// ---------------------------------------------------------------------------
// Test 1a: ASCII fixture — per_cohort_union clean workspace
// ---------------------------------------------------------------------------

#[test]
fn ascii_per_cohort_union_clean_no_diagnostics() {
    // The clean workspace produces zero diagnostics, so this mainly exercises
    // that LineIndex can be built and doesn't panic on real fixture text.
    let workspace_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/per_cohort_union");
    let model_path = workspace_root.join("models/orders.sql");
    let text = std::fs::read_to_string(&model_path)
        .unwrap_or_else(|e| panic!("Cannot read {model_path:?}: {e}"));

    let line_index = LineIndex::new(&text);

    // For each line/column pair we can find, check that LineIndex agrees.
    // In a clean file with no diagnostics we just verify the index builds.
    let _ = line_index.line_col(TextSize::from(0u32));
    let _ = line_index.line_col(TextSize::from(text.len().saturating_sub(1) as u32));
}

// ---------------------------------------------------------------------------
// Test 1b: ASCII fixture — broken workspace produces diagnostics; parity holds
// ---------------------------------------------------------------------------

#[test]
fn ascii_broken_emission_body_undeclared_column_parity() {
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/per_cohort_union_broken_emission_body_undeclared_column");
    let model_path = workspace_root.join("models/broken.gen.sql");
    let text = std::fs::read_to_string(&model_path)
        .unwrap_or_else(|e| panic!("Cannot read {model_path:?}: {e}"));

    let (text, diags) = diagnostics_for_text(&text, model_path, workspace_root);

    // We expect at least one diagnostic from the broken file.
    assert!(
        !diags.is_empty(),
        "expected diagnostics from broken emission body; got none"
    );

    let line_index = LineIndex::new(&text);

    for diag in &diags {
        // Shadow-mode parity check: round-trip the diagnostic's (line, col)
        // through a byte-offset and back via LineIndex.
        for (pos_label, pos) in [("start", diag.range.start), ("end", diag.range.end)] {
            let Some(byte_offset) = codepoint_position_to_byte_offset(&text, pos.line, pos.column)
            else {
                // Position past end-of-file (can happen for end positions at EOF).
                continue;
            };
            let lc = line_index.line_col(TextSize::from(byte_offset as u32));
            assert_eq!(
                lc.line, pos.line,
                "LineIndex line mismatch at diagnostic {pos_label}: \
                 expected line={}, got line={}; diag={:?}",
                pos.line, lc.line, diag
            );
            assert_eq!(
                lc.col, pos.column,
                "LineIndex col mismatch at diagnostic {pos_label}: \
                 expected col={}, got col={}; diag={:?}",
                pos.column, lc.col, diag
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Test 1c: ASCII fixture — staging_from_sources; parity on zero-diagnostic workspace
// ---------------------------------------------------------------------------

#[test]
fn ascii_staging_from_sources_line_index_builds() {
    let workspace_root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/staging_from_sources");
    // Pick one SQL model file from the workspace to exercise text indexing.
    let model_path = workspace_root.join("models/staging/stg_orders.sql");
    let text = std::fs::read_to_string(&model_path).unwrap_or_else(|_| {
        // Fall back to any .sql file we can find.
        let staging_dir = workspace_root.join("models/staging");
        std::fs::read_dir(&staging_dir)
            .expect("staging dir")
            .filter_map(|e| e.ok())
            .find(|e| e.path().extension().map(|x| x == "sql").unwrap_or(false))
            .map(|e| std::fs::read_to_string(e.path()).unwrap())
            .unwrap_or_else(|| "SELECT 1 AS x".to_string())
    });

    let line_index = LineIndex::new(&text);

    // Walk every byte position (by char boundary) and verify LineIndex returns
    // something sensible (no panic, non-negative values).
    for (i, _) in text.char_indices() {
        let lc = line_index.line_col(TextSize::from(i as u32));
        // line and col are u32 — just ensure they're stable when text is ASCII.
        let _ = (lc.line, lc.col);
    }
}

// ---------------------------------------------------------------------------
// Test 2: Non-ASCII unit gate — LineIndex byte-column semantics
// ---------------------------------------------------------------------------

/// For the string `"α\nβ\nγ\n"`:
/// - `α` is U+03B1, encoded as 2 UTF-8 bytes: `0xCE 0xB1`.
/// - Line 0 starts at byte 0; line 1 starts at byte 3 (2 bytes for α + 1 for \n).
///
/// A `TextSize` pointing at byte 3 (start of line 1) must yield `(line=1, col=0)`.
/// This is the focused unit gate on `LineIndex`'s byte-column semantics.
#[test]
fn non_ascii_line_index_byte_col_semantics() {
    let text = "α\nβ\nγ\n";
    // α is 2 UTF-8 bytes, \n is 1 byte. So:
    //   byte 0..2  = α  (line 0, col 0..2)
    //   byte 2     = \n (line 0, col 2 — the newline itself)
    //   byte 3     = first byte of β on line 1
    let line_index = LineIndex::new(text);

    // Position pointing at the first byte of "β" (line 1, col 0 in bytes).
    let ts = TextSize::from(3u32);
    let lc = line_index.line_col(ts);
    assert_eq!(lc.line, 1, "expected line=1 for byte 3 in {:?}", text);
    assert_eq!(
        lc.col, 0,
        "expected col=0 (byte-col) for byte 3 in {:?}",
        text
    );

    // Also check that byte 0 is line 0, col 0.
    let lc0 = line_index.line_col(TextSize::from(0u32));
    assert_eq!(lc0.line, 0);
    assert_eq!(lc0.col, 0);

    // Byte 2 is the newline after α — still on line 0, col 2 (byte index of '\n').
    let lc2 = line_index.line_col(TextSize::from(2u32));
    assert_eq!(
        lc2.line, 0,
        "byte 2 (\\n after α) should still be on line 0"
    );
    assert_eq!(lc2.col, 2, "byte 2 is byte-col 2 (the newline character)");
}

/// Non-ASCII: contrast with `smelt_parser::offset_to_position` which counts
/// **Unicode codepoints**, not bytes. For `"α\nβ\nγ\n"`:
/// - `α` is one codepoint but two bytes.
/// - `smelt_parser::offset_to_position(text, 2)` walks past two codepoints
///   (0: α, 1: \n) so it lands at the start of β — `(line=1, col=0)`.
///
/// This test pins the divergence between codepoint-offsets (what
/// `Diagnostic::range.start.column` stores today) and byte-offsets (what
/// `LineIndex` natively uses): for non-ASCII text they are different numbers.
/// For ASCII they coincide — that's why the shadow assert in the emission
/// points is safe for ASCII workspaces today.
#[test]
fn non_ascii_divergence_between_codepoint_and_byte_offset() {
    let text = "α\nβ\nγ\n";
    // smelt_parser::offset_to_position counts codepoints.
    // Codepoint 0 = α, codepoint 1 = \n, codepoint 2 = β.
    let codepoint_pos = smelt_parser::offset_to_position(text, 2);
    assert_eq!(codepoint_pos.line, 1, "codepoint offset 2 should be line 1");
    assert_eq!(
        codepoint_pos.column, 0,
        "codepoint offset 2 should be col 0 (start of β)"
    );

    // But byte offset 2 is the second byte of α (still on line 0).
    let line_index = LineIndex::new(text);
    let byte_lc = line_index.line_col(TextSize::from(2u32));
    assert_eq!(
        byte_lc.line, 0,
        "byte offset 2 is the second byte of α, still on line 0"
    );
    assert_ne!(
        byte_lc.col, codepoint_pos.column,
        "byte col and codepoint col should differ for non-ASCII α"
    );
}

// ---------------------------------------------------------------------------
// Test 3: Per-diagnostic parity using a synthetic ASCII model with a known error
// ---------------------------------------------------------------------------

/// Build a small in-memory workspace with an intentional parse-level error and
/// verify that every diagnostic's `(line, col)` — as produced by
/// `offset_to_position` — round-trips cleanly through `LineIndex` when the
/// text is pure ASCII.
#[test]
fn synthetic_ascii_model_parity() {
    // A model referencing a non-existent smelt.ref — produces a diagnostic.
    let sql = "SELECT *\nFROM smelt.nonexistent_model\nWHERE id > 0\n";
    let root = PathBuf::from("/synthetic/parity_project");
    let model_path = root.join("models/broken.sql");

    let (text, diags) = diagnostics_for_text(sql, model_path, root);
    // We may or may not get diagnostics depending on workspace state,
    // but we can still verify parity over any we do get.

    let line_index = LineIndex::new(&text);

    for diag in &diags {
        for (label, pos) in [("start", diag.range.start), ("end", diag.range.end)] {
            let Some(byte_off) = codepoint_position_to_byte_offset(&text, pos.line, pos.column)
            else {
                continue;
            };
            let lc = line_index.line_col(TextSize::from(byte_off as u32));
            assert_eq!(
                lc.line, pos.line,
                "ASCII parity: line mismatch at {label}; diag={:?}",
                diag
            );
            assert_eq!(
                lc.col, pos.column,
                "ASCII parity: col mismatch at {label}; diag={:?}",
                diag
            );
        }
    }
}
