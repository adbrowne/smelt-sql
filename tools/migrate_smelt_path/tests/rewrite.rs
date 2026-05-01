//! Integration tests for the migrate_smelt_path rewrite logic.
//! These are the TDD tests written *before* the migration is run.

use migrate_smelt_path::rewrite_content;

/// Test 1: smelt.ref(...) → smelt.models.<name>
#[test]
fn rewrites_smelt_ref_to_path_form() {
    let input = "SELECT * FROM smelt.ref('users')";
    let expected = "SELECT * FROM smelt.models.users";
    assert_eq!(rewrite_content(input), expected);
}

/// Test 2: smelt.source(...) → smelt.sources.<path>
#[test]
fn rewrites_smelt_source_to_path_form() {
    let input = "FROM smelt.source('raw.events')";
    let expected = "FROM smelt.sources.raw.events";
    assert_eq!(rewrite_content(input), expected);
}

/// Test 3: smelt.fn.<name>( → smelt.functions.<name>(
#[test]
fn rewrites_smelt_fn_to_path_form() {
    let input = "SELECT smelt.fn.safe_divide(x, y) FROM t";
    let expected = "SELECT smelt.functions.safe_divide(x, y) FROM t";
    assert_eq!(rewrite_content(input), expected);
}

/// Comment lines must not be rewritten.
#[test]
fn preserves_comment_lines() {
    let input = "-- smelt.ref('keep_this') unchanged\nSELECT * FROM smelt.ref('users')";
    let expected = "-- smelt.ref('keep_this') unchanged\nSELECT * FROM smelt.models.users";
    assert_eq!(rewrite_content(input), expected);
}

/// Double-quoted ref should also be rewritten.
#[test]
fn rewrites_double_quoted_ref() {
    let input = r#"SELECT * FROM smelt.ref("orders")"#;
    let expected = "SELECT * FROM smelt.models.orders";
    assert_eq!(rewrite_content(input), expected);
}

/// File-level round-trip: migrate_file writes back only when content changes.
#[test]
fn migrate_file_writes_back_only_when_changed() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.sql");
    std::fs::write(&file, "SELECT * FROM smelt.ref('users')").unwrap();

    let changed = migrate_smelt_path::migrate_file(&file).unwrap();
    assert!(changed, "file should have been modified");

    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "SELECT * FROM smelt.models.users");

    // Second call — no more legacy syntax, nothing to change.
    let changed_again = migrate_smelt_path::migrate_file(&file).unwrap();
    assert!(!changed_again, "file should be unchanged on second call");
}
