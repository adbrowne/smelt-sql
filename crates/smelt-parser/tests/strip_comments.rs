//! Tests for `smelt_parser::strip_sql_comments`.
//!
//! `strip_sql_comments` is a token-level utility: it lexes the input and
//! reproduces every non-`COMMENT` token's original text verbatim, dropping
//! only `COMMENT` tokens. See `docs/specs/ui_model_diagnostics.md`
//! §Semantics "Comment stripping" and §Constraints & Invariants for the
//! idempotence / whitespace-preservation contract this guards.

use smelt_parser::strip_sql_comments;

#[test]
fn strips_line_comment_preserves_layout() {
    let input = "-- leading comment\nSELECT 1 -- trailing comment\nFROM t\n";
    let result = strip_sql_comments(input);

    assert!(!result.contains("comment"));
    // Surrounding whitespace/newlines are preserved: the comment text is
    // removed but the newline that followed it (and the rest of the line
    // structure) survives untouched.
    assert_eq!(result, "\nSELECT 1 \nFROM t\n");
}

#[test]
fn strips_nested_block_comment() {
    let input = "SELECT 1 /* outer /* inner */ still outer */ FROM t";
    let result = strip_sql_comments(input);

    assert!(!result.contains("outer"));
    assert!(!result.contains("inner"));
    assert_eq!(result, "SELECT 1  FROM t");
}

#[test]
fn preserves_comment_like_text_inside_string_literal() {
    let input = "SELECT '-- not a comment' AS a, '/* not a comment */' AS b FROM t";
    let result = strip_sql_comments(input);

    // The lexer tokenizes string literals as a single token distinct from
    // COMMENT, so comment-like text inside a string literal must survive
    // byte-for-byte.
    assert_eq!(result, input);
}

#[test]
fn idempotent() {
    let input = "-- header\nSELECT 1 /* inline */ AS a -- trailing\nFROM t\n";
    let once = strip_sql_comments(input);
    let twice = strip_sql_comments(&once);

    assert_eq!(once, twice);
}

#[test]
fn real_fixture_examples_timeseries() {
    // examples/timeseries/models/cube_metrics.sql has both a file-header
    // line-comment block and a trailing same-line comment
    // (`-- smelt:cube_split`), with no frontmatter block, making it a good
    // real-fixture case for this phase.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/timeseries/models/cube_metrics.sql"
    );
    let input = std::fs::read_to_string(path).expect("fixture model file should exist");
    let result = strip_sql_comments(&input);

    assert!(
        !result.contains("Cube split optimization"),
        "header comment should be stripped"
    );
    assert!(
        !result.contains("smelt:cube_split"),
        "trailing comment should be stripped"
    );
    // Non-comment SQL content must survive untouched.
    assert!(result.contains("GROUP BY 1, 2"));
    assert!(result.contains("FROM smelt.sources.raw.events"));

    let expected = "\n\nSELECT\n    date_trunc('day', event_timestamp) as event_date,\n    event_type,\n    COUNT(DISTINCT user_id) as unique_users,\n    COUNT(DISTINCT event_id) as unique_events,\n    COUNT(*) as total_events\nFROM smelt.sources.raw.events\nGROUP BY 1, 2 \n\n";
    assert_eq!(result, expected);
}
