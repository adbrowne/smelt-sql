//! Tests for lambda-parameter rename support in the LSP.
//!
//! Each test calls the pure helper `rename_lambda_param` (and
//! `prepare_rename_lambda_param`) from `smelt_lsp::rename_lambda` and
//! verifies the resulting span lists and error messages.

use smelt_lsp::rename_lambda::{
    prepare_rename_lambda_param, rename_lambda_param, RenameLambdaError, RenameLambdaResult,
};
use smelt_parser::{ast::File as AstFile, parse};

// ───────────────────────── helpers ─────────────────────────────────────────

/// Parse the given SQL, return the root File AST node.
fn parse_file(sql: &str) -> AstFile {
    let parsed = parse(sql);
    AstFile::cast(parsed.syntax()).expect("root is a File")
}

/// Apply a list of `(start_byte, end_byte, new_text)` edits to the source and
/// return the resulting string.  Edits must be non-overlapping and sorted by
/// start offset descending (so they don't shift earlier offsets).
fn apply_edits(src: &str, mut edits: Vec<(usize, usize, String)>) -> String {
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut result = src.to_string();
    for (start, end, new_text) in edits {
        result.replace_range(start..end, &new_text);
    }
    result
}

// ─────────────────────────── tests ─────────────────────────────────────────

/// `xs |> map(fn x => x + 1)` — rename `x` → `n`.
/// Expect exactly two edits: the binder and the one use in the body.
#[test]
fn rename_lambda_param_single_use() {
    let sql = "SELECT xs |> map(fn x => x + 1)";
    let file = parse_file(sql);

    // Position the cursor on the binder `x` (after "fn ").
    // "SELECT xs |> map(fn " is 20 chars, so cursor offset = 20.
    let cursor_offset = 20usize;

    let result = rename_lambda_param(&file, sql, cursor_offset, "n");
    let edits = match result {
        Ok(RenameLambdaResult::Edits(e)) => e,
        other => panic!("expected Edits, got {:?}", other),
    };

    assert_eq!(
        edits.len(),
        2,
        "expected 2 edits (binder + one body use), got {}",
        edits.len()
    );

    let resulting_text = apply_edits(sql, edits);
    assert!(
        resulting_text.contains("fn n => n + 1"),
        "expected `fn n => n + 1` in `{}`, got `{}`",
        sql,
        resulting_text,
    );
}

/// `xs |> map(fn x => x * x + 1)` — rename `x` → `y`.
/// Expect three edits: binder + two body uses.
#[test]
fn rename_lambda_param_multiple_uses() {
    let sql = "SELECT xs |> map(fn x => x * x + 1)";
    let file = parse_file(sql);

    // Cursor on the binder `x` (at offset 20).
    let cursor_offset = 20usize;

    let result = rename_lambda_param(&file, sql, cursor_offset, "y");
    let edits = match result {
        Ok(RenameLambdaResult::Edits(e)) => e,
        other => panic!("expected Edits, got {:?}", other),
    };

    assert_eq!(
        edits.len(),
        3,
        "expected 3 edits (binder + two body uses), got {}: {:?}",
        edits.len(),
        edits
    );

    let resulting_text = apply_edits(sql, edits);
    assert!(
        resulting_text.contains("fn y => y * y + 1"),
        "expected `fn y => y * y + 1` in result, got `{}`",
        resulting_text,
    );
}

/// Nested lambdas with the same parameter name.  Renaming the outer `x` must
/// NOT touch the inner lambda's binder or body.
#[test]
fn rename_lambda_param_inner_shadowing_preserved() {
    // outer lambda: fn x => ..., inner lambda: fn x => x.path
    let sql = "SELECT xs |> map(fn x => smelt.models.with_tag('cohort') |> map(fn x => x.path))";
    let file = parse_file(sql);

    // Cursor on the outer binder `x` at offset 20 (after "SELECT xs |> map(fn ").
    let cursor_offset = 20usize;

    let result = rename_lambda_param(&file, sql, cursor_offset, "item");
    let edits = match result {
        Ok(RenameLambdaResult::Edits(e)) => e,
        other => panic!("expected Edits, got {:?}", other),
    };

    // Only the outer binder should be renamed; the outer lambda has zero uses
    // of `x` in its body before the inner lambda re-shadows it.
    assert_eq!(
        edits.len(),
        1,
        "expected 1 edit (outer binder only), got {}: {:?}",
        edits.len(),
        edits
    );

    // Verify the inner lambda's `x` tokens are untouched.
    let resulting_text = apply_edits(sql, edits);
    assert!(
        resulting_text.contains("fn x => x.path"),
        "inner lambda should still have `fn x => x.path`, got `{}`",
        resulting_text
    );
    assert!(
        resulting_text.contains("fn item =>"),
        "outer lambda binder should be renamed to `item`, got `{}`",
        resulting_text
    );
}

/// Renaming to `1abc` (invalid SQL identifier) must return an error.
#[test]
fn rename_lambda_param_rejects_invalid_identifier() {
    let sql = "SELECT xs |> map(fn x => x + 1)";
    let file = parse_file(sql);
    let cursor_offset = 20usize;

    let result = rename_lambda_param(&file, sql, cursor_offset, "1abc");
    assert!(
        matches!(result, Err(RenameLambdaError::InvalidIdentifier(_))),
        "expected InvalidIdentifier error, got {:?}",
        result
    );
}

/// Renaming to `if` (reserved meta-keyword) must return an error.
#[test]
fn rename_lambda_param_rejects_keyword_collision() {
    let sql = "SELECT xs |> map(fn x => x + 1)";
    let file = parse_file(sql);
    let cursor_offset = 20usize;

    let result = rename_lambda_param(&file, sql, cursor_offset, "if");
    assert!(
        matches!(result, Err(RenameLambdaError::ReservedKeyword(_))),
        "expected ReservedKeyword error, got {:?}",
        result
    );
}

/// Renaming where the new name would shadow an outer binder that is already
/// referenced in the inner body must return an error.
#[test]
fn rename_lambda_param_rejects_shadowing_outer_binder() {
    // outer: fn outer_var => ..., inner: fn y => outer_var + y
    // If we rename inner `y` → `outer_var`, `outer_var` in the inner body
    // would be ambiguous (it now refers to the inner binder, shadowing outer).
    let sql = "SELECT xs |> map(fn outer_var => ys |> map(fn y => outer_var + y))";
    let file = parse_file(sql);

    // Cursor on the inner binder `y` (after "fn outer_var => ys |> map(fn ")
    // "SELECT xs |> map(fn outer_var => ys |> map(fn " is 47 chars.
    let cursor_offset = 47usize;

    let result = rename_lambda_param(&file, sql, cursor_offset, "outer_var");
    assert!(
        matches!(result, Err(RenameLambdaError::ShadowsOuterBinder(_))),
        "expected ShadowsOuterBinder error, got {:?}",
        result
    );
}

/// Multi-arg lambda `fn (a, b) => a + b` — rename `a` → `x`.
/// Only the `a` binder and its body use are renamed; `b` is untouched.
#[test]
fn rename_lambda_param_multi_arg() {
    let sql = "SELECT xs |> map(fn (a, b) => a + b)";
    let file = parse_file(sql);

    // Cursor on the `a` binder (after "SELECT xs |> map(fn (")
    // that's offset 21.
    let cursor_offset = 21usize;

    let result = rename_lambda_param(&file, sql, cursor_offset, "x");
    let edits = match result {
        Ok(RenameLambdaResult::Edits(e)) => e,
        other => panic!("expected Edits, got {:?}", other),
    };

    // binder `a` + one body use `a` in `a + b` = 2 edits
    assert_eq!(
        edits.len(),
        2,
        "expected 2 edits (a-binder + a-body use), got {}: {:?}",
        edits.len(),
        edits
    );

    let resulting_text = apply_edits(sql, edits);
    assert!(
        resulting_text.contains("fn (x, b) => x + b"),
        "expected `fn (x, b) => x + b`, got `{}`",
        resulting_text
    );
}

/// prepare-rename on a lambda binder returns the binder's byte range and
/// the current parameter name as placeholder.
#[test]
fn prepare_rename_lambda_param_returns_range() {
    let sql = "SELECT xs |> map(fn x => x + 1)";
    let file = parse_file(sql);
    let cursor_offset = 20usize;

    let result = prepare_rename_lambda_param(&file, sql, cursor_offset);
    let (start, end, placeholder) = match result {
        Some(r) => r,
        None => panic!("expected Some, got None"),
    };

    assert_eq!(placeholder, "x", "placeholder must be the param name");
    // The binder is the single character `x`; byte range must have length 1.
    assert_eq!(
        end - start,
        1,
        "binder `x` must span 1 byte, got {}",
        end - start
    );
    assert_eq!(&sql[start..end], "x", "text at range must be `x`");
}

/// prepare-rename on a non-binder token (an operator `=>`) must return None.
/// Existing rename targets (CTE, `smelt.<path>`) continue to work via their
/// own handlers and are not affected.
#[test]
fn prepare_rename_on_non_lambda_target_does_not_match() {
    let sql = "SELECT xs |> map(fn x => x + 1)";
    let file = parse_file(sql);

    // Cursor on `=>` (starts at offset 22 — after "SELECT xs |> map(fn x ")
    let cursor_offset = 22usize;

    let result = prepare_rename_lambda_param(&file, sql, cursor_offset);
    assert!(
        result.is_none(),
        "expected None for cursor on `=>`, got Some({:?})",
        result
    );
}
