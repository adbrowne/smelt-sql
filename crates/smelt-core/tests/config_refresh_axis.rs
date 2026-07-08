//! TDD tests for the refresh-axis surface cut: `RefreshStrategy` becomes the
//! trichotomy `Full | Incremental | MaterializedView`, and `grain:` is
//! required whenever `refresh: incremental` is set.
//!
//! Spec oracle: `docs/specs/models.md` §"Refresh axis".

use smelt_core::metadata::{extract_file_metadata, MetadataError};

/// `refresh: incremental` without a sibling `grain:` is a hard error naming
/// the missing key. Frontmatter parsing (`extract_file_metadata`) succeeds —
/// the cross-field check runs in `validate_timeseries`, the same pure
/// semantic-validation function that checks the other refresh/grain-adjacent
/// constraints (`TimeseriesRequiredForBatched`, `KeyedForbidsBatched`, …).
#[test]
fn incremental_requires_grain() {
    let source = r#"---
materialization: table
refresh: incremental
---
SELECT 1 AS n"#;

    let metadata = match extract_file_metadata(source).expect("frontmatter parse must succeed") {
        smelt_core::metadata::FileMetadata::Single { metadata, .. } => *metadata,
        other => panic!("Expected Single variant, got {other:?}"),
    };
    assert_eq!(
        metadata.refresh,
        Some(smelt_core::config::RefreshStrategy::Incremental)
    );
    assert_eq!(metadata.grain, None);

    let err = smelt_core::metadata::validate_timeseries(&metadata, "SELECT 1 AS n")
        .expect_err("`refresh: incremental` without `grain:` must be a hard error");
    assert!(
        matches!(err, MetadataError::GrainRequiredForIncremental),
        "Expected GrainRequiredForIncremental, got: {}",
        err
    );
    let message = err.to_string();
    assert!(
        message.contains("grain"),
        "error must name the missing `grain:` key; got: {message}"
    );
}

/// `grain:` without `refresh: incremental` is also a hard error (grain is
/// only meaningful alongside `refresh: incremental`).
#[test]
fn grain_without_incremental_errors() {
    let metadata = smelt_core::metadata::ModelMetadata {
        materialization: Some(smelt_core::config::Materialization::Table),
        grain: Some(smelt_core::config::Grain::Partition),
        ..Default::default()
    };
    let err = smelt_core::metadata::validate_timeseries(&metadata, "SELECT 1")
        .expect_err("`grain:` without `refresh: incremental` must be a hard error");
    assert!(
        matches!(err, MetadataError::GrainRequiresIncremental),
        "Expected GrainRequiresIncremental, got: {}",
        err
    );
}

/// Each of the removed `refresh:` mode names (`batched`, `keyed`,
/// `cumulative`, `versioned`) hard-errors with a fix-it naming the
/// `refresh: incremental` + `grain:` replacement.
#[test]
fn removed_names_error_with_fixit() {
    for value in ["batched", "keyed", "cumulative", "versioned"] {
        let source = format!("---\nmaterialization: table\nrefresh: {value}\n---\nSELECT 1 AS n");
        let result = extract_file_metadata(&source);
        let err = match result {
            Ok(v) => panic!("`refresh: {value}` must be rejected; got Ok({v:?})"),
            Err(e) => e,
        };
        let message = err.to_string();
        assert!(
            message.contains("refresh: incremental"),
            "error for '{value}' must contain the fix-it `refresh: incremental`; got: {message}"
        );
        assert!(
            message.contains("grain:"),
            "error for '{value}' must contain the fix-it `grain:`; got: {message}"
        );
    }
}
