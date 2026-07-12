//! TDD tests for Phase 2: smelt.check kind, severity, and is_check/is_assertion.

use smelt_core::discovery::ModelFile;
use smelt_core::model_id::ModelId;
use std::path::PathBuf;

fn make_model_file(path: &str, content: &str) -> ModelFile {
    let p = PathBuf::from(path);
    ModelFile {
        name: p.file_stem().unwrap().to_string_lossy().to_string(),
        model_id: ModelId::from_path(p.clone()),
        path: p,
        content: content.to_string(),
        refs: vec![],
        parse_errors: vec![],
        metadata: None,
        kind: smelt_core::discovery::ModelKind::Sql,
        address_segments: vec!["check_file".to_string()],
    }
}

/// A `smelt.check` file: `is_check()` returns true; `is_test()` stays false.
#[test]
fn is_check_via_parser() {
    let check_sql = "smelt.check no_nulls AS (SELECT id FROM smelt.orders WHERE id IS NULL)";
    let model = make_model_file("/project/checks/no_nulls.sql", check_sql);

    assert!(
        model.is_check(),
        "is_check() must return true for a smelt.check file"
    );
    assert!(
        !model.is_test(),
        "is_test() must return false for a smelt.check file"
    );
    assert!(
        model.is_assertion(),
        "is_assertion() must return true for a smelt.check file"
    );
}

/// A regular model: `is_check()` and `is_test()` are both false.
#[test]
fn regular_model_is_not_check() {
    let model = make_model_file("/project/models/orders.sql", "SELECT id FROM raw.orders");

    assert!(!model.is_check(), "is_check() must be false for a model");
    assert!(!model.is_test(), "is_test() must be false for a model");
    assert!(
        !model.is_assertion(),
        "is_assertion() must be false for a model"
    );
}

/// A `smelt.test` file: `is_check()` stays false; `is_test()` stays true.
#[test]
fn test_file_is_not_check() {
    let test_sql = "smelt.test my_test AS (SELECT 1)\nPASSING x AS ({a: 1})\nEXPECT ({a: 1})";
    let model = make_model_file("/project/tests/my_test.sql", test_sql);

    assert!(
        !model.is_check(),
        "is_check() must be false for a smelt.test file"
    );
    assert!(
        model.is_test(),
        "is_test() must be true for a smelt.test file"
    );
    assert!(
        model.is_assertion(),
        "is_assertion() must be true for a smelt.test file"
    );
}

/// `severity: warn` frontmatter on a `smelt.check` deserializes correctly.
/// Default is `error`. Unknown value fails.
#[test]
fn check_severity_parses() {
    use smelt_core::metadata::{extract_file_metadata, CheckSeverity, FileMetadata};

    // Default: no frontmatter → severity defaults to Error
    let no_fm = "smelt.check no_nulls AS (SELECT 1 WHERE false)";
    assert!(
        matches!(extract_file_metadata(no_fm), Ok(FileMetadata::Empty)),
        "no frontmatter should give FileMetadata::Empty"
    );

    // severity: warn
    let warn_fm = "---\nseverity: warn\n---\nsmelt.check warn_check AS (SELECT 1 WHERE false)";
    let meta = extract_file_metadata(warn_fm).expect("should parse severity: warn");
    match meta {
        FileMetadata::Single { metadata, .. } => {
            let chk = metadata
                .check
                .as_ref()
                .expect("check config should be present");
            assert_eq!(
                chk.severity,
                CheckSeverity::Warn,
                "severity: warn should deserialize as Warn"
            );
        }
        other => panic!("expected Single metadata, got {other:?}"),
    }

    // severity: error (explicit)
    let error_fm = "---\nseverity: error\n---\nsmelt.check error_check AS (SELECT 1 WHERE false)";
    let meta2 = extract_file_metadata(error_fm).expect("should parse severity: error");
    match meta2 {
        FileMetadata::Single { metadata, .. } => {
            let chk = metadata
                .check
                .as_ref()
                .expect("check config should be present");
            assert_eq!(
                chk.severity,
                CheckSeverity::Error,
                "severity: error should deserialize as Error"
            );
        }
        other => panic!("expected Single metadata, got {other:?}"),
    }

    // severity: bogus → fail-loud: must return Err, never silently default.
    let bad_fm = "---\nseverity: bogus\n---\nsmelt.check bad_check AS (SELECT 1 WHERE false)";
    assert!(
        extract_file_metadata(bad_fm).is_err(),
        "an invalid severity value must return Err, not silently default to Error"
    );
}
