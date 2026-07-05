//! Tests for the `refresh:` frontmatter axis.
//!
//! Spec oracle: `docs/specs/models.md` §"YAML frontmatter keys" and
//! `docs/specs/keyed_models.md` §Surface.

use smelt_core::config::{Materialization, RefreshStrategy};
use smelt_core::metadata::{
    extract_file_metadata, validate_timeseries, FileMetadata, ModelMetadata,
};

// ── refresh: keyed parses ─────────────────────────────────────────────────────

/// `materialization: table` + `refresh: keyed` parses to
/// `RefreshStrategy::Keyed`; a model with no `refresh:` key defaults to
/// `RefreshStrategy::Full` (or `None` in the `Option<RefreshStrategy>` field).
#[test]
fn refresh_keyed_parses() {
    let source = r#"---
materialization: table
refresh: keyed
---
SELECT device_id, user_id, COUNT(*) AS n FROM smelt.events GROUP BY device_id, user_id"#;

    let result = extract_file_metadata(source).expect("should parse");
    match result {
        FileMetadata::Single { metadata, .. } => {
            assert_eq!(metadata.materialization, Some(Materialization::Table));
            assert_eq!(metadata.refresh, Some(RefreshStrategy::Keyed));
            // is_keyed() must return true for the new surface
            assert!(
                metadata.is_keyed(),
                "is_keyed() must be true for refresh: keyed"
            );
        }
        _ => panic!("Expected Single variant"),
    }
}

/// `refresh: cumulative` is a hard error pointing at the renamed value —
/// not a silent alias.
#[test]
fn refresh_cumulative_is_hard_error_pointing_at_keyed() {
    let source = r#"---
materialization: table
refresh: cumulative
---
SELECT device_id, user_id, COUNT(*) AS n FROM smelt.events GROUP BY device_id, user_id"#;

    let err = extract_file_metadata(source).expect_err("`refresh: cumulative` must be rejected");
    let message = err.to_string();
    assert!(
        message.contains("`refresh: cumulative` is now `refresh: keyed`"),
        "error must contain the exact pointer message; got: {message}"
    );
}

/// A model with no `refresh:` key has `None` refresh — not keyed.
#[test]
fn refresh_absent_is_full() {
    let source = r#"---
materialization: table
---
SELECT 1 AS n"#;

    let result = extract_file_metadata(source).expect("should parse");
    match result {
        FileMetadata::Single { metadata, .. } => {
            assert_eq!(metadata.refresh, None);
            assert!(
                !metadata.is_keyed(),
                "is_keyed() must be false when refresh: is absent"
            );
        }
        _ => panic!("Expected Single variant"),
    }
}

/// `refresh: full` explicitly — same as absent.
#[test]
fn refresh_full_is_not_keyed() {
    let source = r#"---
materialization: table
refresh: full
---
SELECT 1 AS n"#;

    let result = extract_file_metadata(source).expect("should parse");
    match result {
        FileMetadata::Single { metadata, .. } => {
            assert_eq!(metadata.refresh, Some(RefreshStrategy::Full));
            assert!(!metadata.is_keyed(), "refresh: full must not be keyed");
        }
        _ => panic!("Expected Single variant"),
    }
}

// ── cumulative_aggregate is now an unknown value ──────────────────────────────

/// `materialization: cumulative_aggregate` must fail to deserialize with a clear
/// unknown-value error now that the variant has been removed.
/// The opt-in is `materialization: table` + `refresh: keyed`.
#[test]
fn cumulative_aggregate_materialization_rejected() {
    let source = r#"---
materialization: cumulative_aggregate
---
SELECT device_id, user_id, COUNT(*) AS n FROM smelt.events GROUP BY device_id, user_id"#;

    let result = smelt_core::metadata::extract_file_metadata(source);
    assert!(
        result.is_err(),
        "`materialization: cumulative_aggregate` must fail to deserialize — \
         the variant has been removed. Use `materialization: table` + `refresh: keyed` instead."
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("cumulative_aggregate") || err.contains("Invalid materialization"),
        "error must mention the invalid value; got: {err}"
    );
}

/// `refresh: latest_value` and `refresh: accumulating_snapshot` remain
/// unknown-value errors — the keyed rename does not introduce them as
/// aliases for `refresh: keyed`.
#[test]
fn refresh_latest_value_and_accumulating_snapshot_remain_unknown() {
    for value in ["latest_value", "accumulating_snapshot"] {
        let source = format!("---\nmaterialization: table\nrefresh: {value}\n---\nSELECT 1 AS n");
        let result = extract_file_metadata(&source);
        assert!(
            result.is_err(),
            "`refresh: {value}` must still be rejected as unknown"
        );
    }
}

// ── refresh: keyed forbids timeseries: and batched: ───────────────────────────

/// `refresh: keyed` + `timeseries:` → `KeyedForbidsTimeseries`.
#[test]
fn refresh_keyed_forbids_timeseries() {
    use smelt_core::config::{Granularity, TimeseriesConfig};
    use smelt_core::metadata::MetadataError;

    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        refresh: Some(RefreshStrategy::Keyed),
        timeseries: Some(TimeseriesConfig {
            event_time_column: "ts".to_string(),
            partition_column: "dt".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        ..Default::default()
    };
    let err = validate_timeseries(&metadata, "SELECT dt FROM foo")
        .expect_err("refresh: keyed + timeseries: must error");
    assert!(
        matches!(err, MetadataError::KeyedForbidsTimeseries),
        "Expected KeyedForbidsTimeseries, got: {}",
        err
    );
}

/// `refresh: keyed` + `batched:` → `KeyedForbidsBatched`.
#[test]
fn refresh_keyed_forbids_incremental() {
    use smelt_core::config::{BatchedConfig, BatchedSafetyOverrides};
    use smelt_core::metadata::MetadataError;

    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        refresh: Some(RefreshStrategy::Keyed),
        batched: Some(BatchedConfig {
            unique_key: vec![],
            nondeterministic_columns: vec![],
            safety_overrides: BatchedSafetyOverrides::default(),
        }),
        ..Default::default()
    };
    let err = validate_timeseries(&metadata, "SELECT * FROM foo")
        .expect_err("refresh: keyed + batched: must error");
    assert!(
        matches!(err, MetadataError::KeyedForbidsBatched),
        "Expected KeyedForbidsBatched, got: {}",
        err
    );
}

// ── refresh: batched selector + batched: block ────────────────────────────────

/// `refresh: batched` deserialises to `RefreshStrategy::Batched`.
#[test]
fn refresh_batched_parses() {
    let source = r#"---
materialization: table
refresh: batched
timeseries:
  event_time_column: ts
  partition_column: dt
  granularity: day
---
SELECT dt FROM foo"#;

    let result = extract_file_metadata(source).expect("should parse");
    match result {
        FileMetadata::Single { metadata, .. } => {
            assert_eq!(metadata.refresh, Some(RefreshStrategy::Batched));
        }
        _ => panic!("Expected Single variant"),
    }
}

/// A bare `refresh: foo` still errors listing `batched` among the valid values.
#[test]
fn refresh_unknown_value_lists_batched() {
    let err = serde_yaml::from_str::<RefreshStrategy>("foo")
        .expect_err("unknown refresh value must fail");
    let message = err.to_string();
    assert!(
        message.contains("batched"),
        "error must list 'batched' among valid refresh values; got: {}",
        message
    );
}

/// `refresh: batched` + `timeseries:` validates cleanly.
#[test]
fn refresh_batched_with_timeseries_is_valid() {
    use smelt_core::config::{Granularity, TimeseriesConfig};

    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        refresh: Some(RefreshStrategy::Batched),
        timeseries: Some(TimeseriesConfig {
            event_time_column: "ts".to_string(),
            partition_column: "dt".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        ..Default::default()
    };
    validate_timeseries(&metadata, "SELECT dt FROM foo")
        .expect("refresh: batched + timeseries: must validate");
}

/// `refresh: batched` without `timeseries:` → `TimeseriesRequiredForBatched`.
#[test]
fn refresh_batched_without_timeseries_errors() {
    use smelt_core::metadata::MetadataError;

    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        refresh: Some(RefreshStrategy::Batched),
        ..Default::default()
    };
    let err = validate_timeseries(&metadata, "SELECT dt FROM foo")
        .expect_err("refresh: batched without timeseries: must error");
    assert!(
        matches!(err, MetadataError::TimeseriesRequiredForBatched),
        "Expected TimeseriesRequiredForBatched, got: {}",
        err
    );
}

/// A `batched:` block without `refresh: batched` is a hard error.
#[test]
fn batched_block_without_refresh_batched_errors() {
    use smelt_core::config::BatchedConfig;
    use smelt_core::metadata::MetadataError;

    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        batched: Some(BatchedConfig::default()),
        ..Default::default()
    };
    let err = validate_timeseries(&metadata, "SELECT 1")
        .expect_err("batched: without refresh: batched must error");
    assert!(
        matches!(err, MetadataError::BatchedRequiresRefreshBatched),
        "Expected BatchedRequiresRefreshBatched, got: {}",
        err
    );
}

/// A model declaring the retired `incremental:` block is a hard error naming
/// `refresh: batched` as the replacement — the hard-cut has no dual-accept
/// deprecation window.
#[test]
fn legacy_incremental_block_is_hard_cut() {
    let source = r#"---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: ts
  partition_column: dt
  granularity: day
---
SELECT dt FROM foo"#;

    let err =
        extract_file_metadata(source).expect_err("declaring incremental: must be a hard error");
    let message = err.to_string();
    assert!(
        message.contains("refresh: batched"),
        "error must name refresh: batched as the replacement; got: {}",
        message
    );
}

// ── view + refresh: keyed is a warning (no error) ─────────────────────────────

/// `view` + `refresh: keyed` emits an advisory warning but does NOT
/// produce a hard error — the config is ignored and the model parses cleanly.
/// (Mirrors the existing `view` + `incremental` treatment.)
#[test]
fn refresh_on_view_is_warning() {
    let metadata = ModelMetadata {
        materialization: Some(Materialization::View),
        refresh: Some(RefreshStrategy::Keyed),
        ..Default::default()
    };
    // Must not error — warning is advisory
    validate_timeseries(&metadata, "SELECT 1")
        .expect("view + refresh: keyed must not be a hard error (only a warning)");
}

// ── ephemeral + refresh: keyed is a hard error ────────────────────────────────

/// `ephemeral` + `refresh: keyed` must be a hard error.
/// Ephemeral models have no persisted output to merge into, so the
/// combination is nonsensical. Mirrors the existing `ephemeral` +
/// `incremental:` treatment (hard error, not a warning).
#[test]
fn refresh_on_ephemeral_is_error() {
    use smelt_core::metadata::MetadataError;

    let metadata = ModelMetadata {
        materialization: Some(Materialization::Ephemeral),
        refresh: Some(RefreshStrategy::Keyed),
        ..Default::default()
    };
    let err = validate_timeseries(&metadata, "SELECT 1")
        .expect_err("ephemeral + refresh: keyed must be a hard error");
    assert!(
        matches!(err, MetadataError::MalformedTimeseries { .. }),
        "Expected MalformedTimeseries, got: {}",
        err
    );
}
