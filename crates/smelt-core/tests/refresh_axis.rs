//! Tests for the `refresh:` frontmatter axis.
//!
//! Spec oracle: `docs/specs/models.md` §"YAML frontmatter keys" and
//! `docs/specs/cumulative_aggregate.md` §Surface.

use smelt_core::config::{Materialization, RefreshStrategy};
use smelt_core::metadata::{
    extract_file_metadata, validate_timeseries, FileMetadata, ModelMetadata,
};

// ── refresh: cumulative parses ────────────────────────────────────────────────

/// `materialization: table` + `refresh: cumulative` parses to
/// `RefreshStrategy::Cumulative`; a model with no `refresh:` key defaults to
/// `RefreshStrategy::Full` (or `None` in the `Option<RefreshStrategy>` field).
#[test]
fn refresh_cumulative_parses() {
    let source = r#"---
materialization: table
refresh: cumulative
---
SELECT device_id, user_id, COUNT(*) AS n FROM smelt.events GROUP BY device_id, user_id"#;

    let result = extract_file_metadata(source).expect("should parse");
    match result {
        FileMetadata::Single { metadata, .. } => {
            assert_eq!(metadata.materialization, Some(Materialization::Table));
            assert_eq!(metadata.refresh, Some(RefreshStrategy::Cumulative));
            // is_cumulative() must return true for the new surface
            assert!(
                metadata.is_cumulative(),
                "is_cumulative() must be true for refresh: cumulative"
            );
        }
        _ => panic!("Expected Single variant"),
    }
}

/// A model with no `refresh:` key has `None` refresh — not cumulative.
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
                !metadata.is_cumulative(),
                "is_cumulative() must be false when refresh: is absent"
            );
        }
        _ => panic!("Expected Single variant"),
    }
}

/// `refresh: full` explicitly — same as absent.
#[test]
fn refresh_full_is_not_cumulative() {
    let source = r#"---
materialization: table
refresh: full
---
SELECT 1 AS n"#;

    let result = extract_file_metadata(source).expect("should parse");
    match result {
        FileMetadata::Single { metadata, .. } => {
            assert_eq!(metadata.refresh, Some(RefreshStrategy::Full));
            assert!(
                !metadata.is_cumulative(),
                "refresh: full must not be cumulative"
            );
        }
        _ => panic!("Expected Single variant"),
    }
}

// ── cumulative_aggregate is now an unknown value ──────────────────────────────

/// `materialization: cumulative_aggregate` must fail to deserialize with a clear
/// unknown-value error now that the variant has been removed.
/// The opt-in is `materialization: table` + `refresh: cumulative`.
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
         the variant has been removed. Use `materialization: table` + `refresh: cumulative` instead."
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("cumulative_aggregate") || err.contains("Invalid materialization"),
        "error must mention the invalid value; got: {err}"
    );
}

// ── refresh: cumulative forbids timeseries: and incremental: ─────────────────

/// `refresh: cumulative` + `timeseries:` → `CumulativeForbidsTimeseries`.
#[test]
fn refresh_cumulative_forbids_timeseries() {
    use smelt_core::config::{Granularity, TimeseriesConfig};
    use smelt_core::metadata::MetadataError;

    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        refresh: Some(RefreshStrategy::Cumulative),
        timeseries: Some(TimeseriesConfig {
            event_time_column: "ts".to_string(),
            partition_column: "dt".to_string(),
            granularity: Granularity::Day,
            week_start: None,
        }),
        ..Default::default()
    };
    let err = validate_timeseries(&metadata, "SELECT dt FROM foo")
        .expect_err("refresh: cumulative + timeseries: must error");
    assert!(
        matches!(err, MetadataError::CumulativeForbidsTimeseries),
        "Expected CumulativeForbidsTimeseries, got: {}",
        err
    );
}

/// `refresh: cumulative` + `batched:` → `CumulativeForbidsBatched`.
#[test]
fn refresh_cumulative_forbids_incremental() {
    use smelt_core::config::{BatchedConfig, BatchedSafetyOverrides};
    use smelt_core::metadata::MetadataError;

    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        refresh: Some(RefreshStrategy::Cumulative),
        batched: Some(BatchedConfig {
            unique_key: vec![],
            safety_overrides: BatchedSafetyOverrides::default(),
        }),
        ..Default::default()
    };
    let err = validate_timeseries(&metadata, "SELECT * FROM foo")
        .expect_err("refresh: cumulative + batched: must error");
    assert!(
        matches!(err, MetadataError::CumulativeForbidsBatched),
        "Expected CumulativeForbidsBatched, got: {}",
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

// ── view + refresh: cumulative is a warning (no error) ───────────────────────

/// `view` + `refresh: cumulative` emits an advisory warning but does NOT
/// produce a hard error — the config is ignored and the model parses cleanly.
/// (Mirrors the existing `view` + `incremental` treatment.)
#[test]
fn refresh_on_view_is_warning() {
    let metadata = ModelMetadata {
        materialization: Some(Materialization::View),
        refresh: Some(RefreshStrategy::Cumulative),
        ..Default::default()
    };
    // Must not error — warning is advisory
    validate_timeseries(&metadata, "SELECT 1")
        .expect("view + refresh: cumulative must not be a hard error (only a warning)");
}

// ── ephemeral + refresh: cumulative is a hard error ───────────────────────────

/// `ephemeral` + `refresh: cumulative` must be a hard error.
/// Ephemeral models have no persisted output to accumulate into, so the
/// combination is nonsensical. Mirrors the existing `ephemeral` +
/// `incremental:` treatment (hard error, not a warning).
#[test]
fn refresh_on_ephemeral_is_error() {
    use smelt_core::metadata::MetadataError;

    let metadata = ModelMetadata {
        materialization: Some(Materialization::Ephemeral),
        refresh: Some(RefreshStrategy::Cumulative),
        ..Default::default()
    };
    let err = validate_timeseries(&metadata, "SELECT 1")
        .expect_err("ephemeral + refresh: cumulative must be a hard error");
    assert!(
        matches!(err, MetadataError::MalformedTimeseries { .. }),
        "Expected MalformedTimeseries, got: {}",
        err
    );
}
