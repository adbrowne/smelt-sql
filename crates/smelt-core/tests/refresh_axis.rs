//! Tests for the `refresh:` frontmatter axis.
//!
//! Spec oracle: `docs/specs/models.md` §"YAML frontmatter keys" and
//! `docs/specs/incremental_models.md` §Surface.

use smelt_core::config::{Grain, Materialization, RefreshStrategy};
use smelt_core::metadata::{
    extract_file_metadata, validate_timeseries, FileMetadata, ModelMetadata,
};

// ── refresh: incremental + grain: key parses ──────────────────────────────────

/// `materialization: table` + `refresh: incremental` + `grain: key` parses to
/// `RefreshStrategy::Incremental` + `Grain::Key`; a model with no `refresh:`
/// key defaults to `RefreshStrategy::Full` (or `None` in the
/// `Option<RefreshStrategy>` field).
#[test]
fn refresh_keyed_parses() {
    let source = r#"---
materialization: table
refresh: incremental
grain: key
---
SELECT device_id, user_id, COUNT(*) AS n FROM smelt.events GROUP BY device_id, user_id"#;

    let result = extract_file_metadata(source).expect("should parse");
    match result {
        FileMetadata::Single { metadata, .. } => {
            assert_eq!(metadata.materialization, Some(Materialization::Table));
            assert_eq!(metadata.refresh, Some(RefreshStrategy::Incremental));
            assert_eq!(metadata.grain, Some(Grain::Key));
            // is_keyed() must return true for the new surface
            assert!(
                metadata.is_keyed(),
                "is_keyed() must be true for refresh: incremental + grain: key"
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
        message.contains("refresh: incremental") && message.contains("grain:"),
        "error must name the refresh: incremental + grain: replacement; got: {message}"
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
/// The opt-in is `materialization: table` + `refresh: incremental` + `grain: key`.
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
         the variant has been removed. Use `materialization: table` + \
         `refresh: incremental` + `grain: key` instead."
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("cumulative_aggregate") || err.contains("Invalid materialization"),
        "error must mention the invalid value; got: {err}"
    );
}

/// `refresh: latest_value` and `refresh: accumulating_snapshot` remain
/// unknown-value errors — the keyed rename does not introduce them as
/// aliases for `refresh: incremental` + `grain: key`.
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

// ── refresh: incremental + grain: key forbids timeseries: and batched: ───────

/// `refresh: incremental` + `grain: key` + `timeseries:` is no longer
/// rejected at frontmatter validation — whether key temporal locality can
/// be established is decided later, by the locality gate in plan
/// derivation (`smelt_logical::maintenance::locality::establish_locality`),
/// not by this pure frontmatter check
/// (`docs/specs/incremental_models.md` §"Key temporal locality (the
/// time-partitioned output)"). The combination now reaches plan derivation
/// instead of failing here.
#[test]
fn refresh_keyed_with_timeseries_reaches_plan_derivation() {
    use smelt_core::config::{Granularity, TimeseriesConfig};

    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Key),
        timeseries: Some(TimeseriesConfig {
            event_time_column: "ts".to_string(),
            partition_column: "dt".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        ..Default::default()
    };
    assert!(
        validate_timeseries(&metadata, "SELECT dt FROM foo").is_ok(),
        "refresh: incremental + grain: key + timeseries: must reach plan derivation, \
         not fail frontmatter validation"
    );
}

/// `refresh: incremental` + `grain: key` with an internally-folded `batched`
/// block → `BatchedRequiresRefreshBatched`. The literal `batched:` sub-block
/// itself is refused at parse time (before a `ModelMetadata` even exists),
/// so this constructs the internal representation directly to exercise
/// `validate_timeseries`'s pure check — the dedicated `KeyedForbidsBatched`
/// check was removed as unreachable (`is_keyed()` implies
/// `!is_partition_grain()`, a strict subset of what
/// `BatchedRequiresRefreshBatched` already checks).
#[test]
fn refresh_keyed_forbids_incremental() {
    use smelt_core::config::{BatchedConfig, BatchedSafetyOverrides};
    use smelt_core::metadata::MetadataError;

    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Key),
        batched: Some(BatchedConfig {
            unique_key: vec![],
            nondeterministic_columns: vec![],
            safety_overrides: BatchedSafetyOverrides::default(),
        }),
        ..Default::default()
    };
    let err = validate_timeseries(&metadata, "SELECT * FROM foo")
        .expect_err("refresh: incremental + grain: key + batched: must error");
    assert!(
        matches!(err, MetadataError::BatchedRequiresRefreshBatched),
        "Expected BatchedRequiresRefreshBatched, got: {}",
        err
    );
}

// ── refresh: incremental + grain: partition selector + batched: block ────────

/// `refresh: incremental` + `grain: partition` deserialises to
/// `RefreshStrategy::Incremental` + `Grain::Partition`.
#[test]
fn refresh_batched_parses() {
    let source = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: ts
  partition_column: dt
  granularity: day
---
SELECT dt FROM foo"#;

    let result = extract_file_metadata(source).expect("should parse");
    match result {
        FileMetadata::Single { metadata, .. } => {
            assert_eq!(metadata.refresh, Some(RefreshStrategy::Incremental));
            assert_eq!(metadata.grain, Some(Grain::Partition));
        }
        _ => panic!("Expected Single variant"),
    }
}

/// A bare `refresh: foo` still errors listing `incremental` among the valid values.
#[test]
fn refresh_unknown_value_lists_batched() {
    let err = serde_yaml::from_str::<RefreshStrategy>("foo")
        .expect_err("unknown refresh value must fail");
    let message = err.to_string();
    assert!(
        message.contains("incremental"),
        "error must list 'incremental' among valid refresh values; got: {}",
        message
    );
}

/// `refresh: incremental` + `grain: partition` + `timeseries:` validates cleanly.
#[test]
fn refresh_batched_with_timeseries_is_valid() {
    use smelt_core::config::{Granularity, TimeseriesConfig};

    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Partition),
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
        .expect("refresh: incremental + grain: partition + timeseries: must validate");
}

/// `refresh: incremental` + `grain: partition` without `timeseries:` →
/// `TimeseriesRequiredForBatched`.
#[test]
fn refresh_batched_without_timeseries_errors() {
    use smelt_core::metadata::MetadataError;

    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Partition),
        ..Default::default()
    };
    let err = validate_timeseries(&metadata, "SELECT dt FROM foo")
        .expect_err("refresh: incremental + grain: partition without timeseries: must error");
    assert!(
        matches!(err, MetadataError::TimeseriesRequiredForBatched),
        "Expected TimeseriesRequiredForBatched, got: {}",
        err
    );
}

/// A `batched:` block without `refresh: incremental` + `grain: partition` is
/// a hard error.
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
        .expect_err("batched: without refresh: incremental + grain: partition must error");
    assert!(
        matches!(err, MetadataError::BatchedRequiresRefreshBatched),
        "Expected BatchedRequiresRefreshBatched, got: {}",
        err
    );
}

/// A model declaring the retired `incremental:` block is a hard error naming
/// `refresh: incremental` + `grain:` as the replacement — the hard-cut has
/// no dual-accept deprecation window.
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
        message.contains("refresh: incremental") && message.contains("grain:"),
        "error must name refresh: incremental + grain: as the replacement; got: {}",
        message
    );
}

// ── view + refresh: incremental + grain: key is a warning (no error) ─────────

/// `view` + `refresh: incremental` + `grain: key` emits an advisory warning
/// but does NOT produce a hard error — the config is ignored and the model
/// parses cleanly. (Mirrors the existing `view` + `incremental` treatment.)
#[test]
fn refresh_on_view_is_warning() {
    let metadata = ModelMetadata {
        materialization: Some(Materialization::View),
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Key),
        ..Default::default()
    };
    // Must not error — warning is advisory
    validate_timeseries(&metadata, "SELECT 1").expect(
        "view + refresh: incremental + grain: key must not be a hard error (only a warning)",
    );
}

// ── ephemeral + refresh: incremental + grain: key is a hard error ────────────

/// `ephemeral` + `refresh: incremental` + `grain: key` must be a hard error.
/// Ephemeral models have no persisted output to merge into, so the
/// combination is nonsensical. Mirrors the existing `ephemeral` +
/// `incremental:` treatment (hard error, not a warning).
#[test]
fn refresh_on_ephemeral_is_error() {
    use smelt_core::metadata::MetadataError;

    let metadata = ModelMetadata {
        materialization: Some(Materialization::Ephemeral),
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Key),
        ..Default::default()
    };
    let err = validate_timeseries(&metadata, "SELECT 1")
        .expect_err("ephemeral + refresh: incremental + grain: key must be a hard error");
    assert!(
        matches!(err, MetadataError::MalformedTimeseries { .. }),
        "Expected MalformedTimeseries, got: {}",
        err
    );
}

// ── S1: top-level unique_key: is the declared identity fact ──────────────────
//
// Spec oracle: `docs/specs/models.md` §"Refresh axis", §"The Relation
// Contract"; `docs/specs/incremental_models.md` §"The declared shape axis".

/// Top-level `unique_key:` parses in `.sql` frontmatter — both the list form
/// and the single-string sugar form — and via a `smelt.yml` model override,
/// with frontmatter winning when both set it.
#[test]
fn top_level_unique_key_parses() {
    // List form.
    let source = r#"---
materialization: table
refresh: incremental
unique_key: [order_id, line_no]
---
SELECT order_id, line_no FROM foo"#;
    let result = extract_file_metadata(source).expect("list-form unique_key: must parse");
    match result {
        FileMetadata::Single { metadata, .. } => {
            assert_eq!(
                metadata.unique_key,
                Some(vec!["order_id".to_string(), "line_no".to_string()])
            );
        }
        _ => panic!("Expected Single variant"),
    }

    // Single-string sugar form.
    let source = r#"---
materialization: table
refresh: incremental
unique_key: order_id
---
SELECT order_id FROM foo"#;
    let result = extract_file_metadata(source).expect("single-string unique_key: must parse");
    match result {
        FileMetadata::Single { metadata, .. } => {
            assert_eq!(metadata.unique_key, Some(vec!["order_id".to_string()]));
        }
        _ => panic!("Expected Single variant"),
    }

    // `smelt.yml` model override parses the same way.
    use smelt_core::config::{Config, ModelConfig};
    let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  orders:
    unique_key: order_id
"#;
    let config: Config = serde_yaml::from_str(yaml).expect("smelt.yml unique_key: must parse");
    assert_eq!(
        config.get_unique_key("orders"),
        Some(["order_id".to_string()].as_slice())
    );

    // Frontmatter wins when both set it.
    let model_config = ModelConfig {
        materialization: None,
        timeseries: None,
        refresh: None,
        grain: None,
        unique_key: Some(vec!["from_yaml".to_string()]),
        safety_overrides: None,
        batched: None,
        tags: vec![],
        target: None,
        format: None,
    };
    let mut models = std::collections::HashMap::new();
    models.insert("orders".to_string(), model_config);
    let mut config = config;
    config.models = models;
    let frontmatter_meta = ModelMetadata {
        unique_key: Some(vec!["from_frontmatter".to_string()]),
        ..Default::default()
    };
    assert_eq!(
        config.get_unique_key_with_metadata("orders", Some(&frontmatter_meta)),
        Some(["from_frontmatter".to_string()].as_slice())
    );
}

/// Top-level `safety_overrides:` in frontmatter parses into `ModelMetadata`,
/// folding into the internal `batched.safety_overrides` representation every
/// existing safety check already reads (`docs/specs/models.md` §"The Relation
/// Contract"). The `batched.safety_overrides` sub-block spelling this
/// replaces no longer parses at all — see `batched_sub_block_is_hard_refused`.
#[test]
fn top_level_safety_overrides_parses() {
    let top_level_source = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_ts
  partition_column: event_date
  granularity: day
safety_overrides:
  allow_window_functions: true
---
SELECT event_ts, event_date FROM foo"#;
    let result =
        extract_file_metadata(top_level_source).expect("top-level safety_overrides: must parse");
    let top_level_batched = match result {
        FileMetadata::Single { metadata, .. } => {
            assert!(
                metadata.safety_overrides.is_none(),
                "top-level safety_overrides is folded into `batched` during extraction"
            );
            metadata
                .batched
                .clone()
                .expect("safety_overrides folds into an implicit `batched:` block")
        }
        _ => panic!("Expected Single variant"),
    };
    assert!(top_level_batched.safety_overrides.allow_window_functions);
}

/// The retired `batched:` sub-block is a hard parse-time error, regardless of
/// its contents — a `batched.safety_overrides` sub-block naming the exact
/// same fact as the top-level `safety_overrides:` spelling is refused just
/// like any other `batched:` declaration, never silently accepted as an
/// alternate spelling.
#[test]
fn batched_sub_block_is_hard_refused() {
    let source = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_ts
  partition_column: event_date
  granularity: day
batched:
  safety_overrides:
    allow_having: true
---
SELECT event_ts, event_date FROM foo"#;
    let err = extract_file_metadata(source)
        .expect_err("the batched: sub-block must be refused regardless of contents");
    let message = err.to_string();
    assert!(
        message.contains("safety_overrides") && message.contains("allow_having"),
        "fix-it must name safety_overrides: and the caller's own declared flag; got: {message}"
    );
}

/// Top-level `safety_overrides:` also parses as a `smelt.yml` model override
/// (`ModelConfig::safety_overrides`), folded into the effective `batched:`
/// block returned by `Config::get_incremental_with_metadata` exactly like the
/// frontmatter spelling — and SQL frontmatter's own top-level (or sub-block)
/// spelling wins wholesale over the smelt.yml one when both set it, mirroring
/// `unique_key:`'s precedence rule.
#[test]
fn top_level_safety_overrides_parses_in_smelt_yml() {
    use smelt_core::config::{BatchedSafetyOverrides, Config};

    let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  daily_revenue:
    materialization: table
    refresh: incremental
    grain: partition
    timeseries:
      event_time_column: event_ts
      partition_column: event_date
      granularity: day
    safety_overrides:
      allow_window_functions: true
"#;
    let config: Config =
        serde_yaml::from_str(yaml).expect("smelt.yml top-level safety_overrides: must parse");

    let batched = config
        .get_incremental_with_metadata("daily_revenue", None)
        .expect("selected model returns Some(batched)");
    assert!(
        batched.safety_overrides.allow_window_functions,
        "smelt.yml top-level safety_overrides: must fold into the effective batched: block"
    );

    // SQL frontmatter wins wholesale over the smelt.yml top-level spelling.
    let frontmatter_meta = ModelMetadata {
        refresh: Some(smelt_core::config::RefreshStrategy::Incremental),
        grain: Some(smelt_core::config::Grain::Partition),
        timeseries: Some(smelt_core::config::TimeseriesConfig {
            event_time_column: "event_ts".to_string(),
            partition_column: "event_date".to_string(),
            granularity: smelt_core::config::Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        batched: Some(smelt_core::config::BatchedConfig {
            safety_overrides: BatchedSafetyOverrides {
                allow_having: true,
                ..Default::default()
            },
            ..Default::default()
        }),
        ..Default::default()
    };
    let batched = config
        .get_incremental_with_metadata("daily_revenue", Some(&frontmatter_meta))
        .expect("selected model returns Some(batched)");
    assert!(
        !batched.safety_overrides.allow_window_functions,
        "frontmatter's batched: block must win wholesale over the smelt.yml top-level spelling"
    );
    assert!(batched.safety_overrides.allow_having);
}

/// Declaring both the top-level `safety_overrides:` key and a non-default
/// `batched.safety_overrides` sub-block on the same `smelt.yml` model entry
/// is a conflict error, mirroring the SQL frontmatter refusal — never silent
/// precedence between the two spellings.
#[test]
fn top_level_safety_overrides_conflicts_with_smelt_yml_batched_sub_block() {
    use smelt_core::config::Config;

    let yaml = r#"
name: test_project
version: 1
targets:
  dev:
    type: duckdb
    database: test.duckdb
    schema: main
models:
  daily_revenue:
    materialization: table
    refresh: incremental
    grain: partition
    timeseries:
      event_time_column: event_ts
      partition_column: event_date
      granularity: day
    safety_overrides:
      allow_window_functions: true
    batched:
      safety_overrides:
        allow_having: true
"#;
    let config: Config = serde_yaml::from_str(yaml).expect("smelt.yml must parse structurally");
    let errors = config.validate_model_configs(&std::collections::HashMap::new());
    assert!(
        errors
            .iter()
            .any(|(name, msg)| name == "daily_revenue" && msg.contains("safety_overrides")),
        "expected a safety_overrides double-declaration error, got {errors:?}"
    );
}

/// `refresh: incremental` is admitted on the shape-defining facts alone — no
/// `grain:` required: a declared `unique_key:` (identity, no clock) derives
/// the key shape; a declared `timeseries:` (clock, no identity) derives the
/// partition shape; neither fact declared is the hard error naming what's
/// missing.
#[test]
fn incremental_admitted_on_facts_alone() {
    use smelt_core::config::Grain;

    // Identity alone → key shape, no grain: written.
    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        refresh: Some(RefreshStrategy::Incremental),
        unique_key: Some(vec!["order_id".to_string()]),
        ..Default::default()
    };
    validate_timeseries(&metadata, "SELECT order_id FROM foo")
        .expect("refresh: incremental + unique_key: alone must be admitted");
    assert_eq!(
        metadata.resolved_grain(),
        Some(Grain::Key),
        "identity alone must derive grain: key"
    );

    // Clock alone → partition shape, no grain: written.
    use smelt_core::config::{Granularity, TimeseriesConfig};
    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        refresh: Some(RefreshStrategy::Incremental),
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
        .expect("refresh: incremental + timeseries: alone must be admitted");
    assert_eq!(
        metadata.resolved_grain(),
        Some(Grain::Partition),
        "clock alone must derive grain: partition"
    );

    // Neither fact declared → hard error naming the missing facts.
    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        refresh: Some(RefreshStrategy::Incremental),
        ..Default::default()
    };
    use smelt_core::metadata::MetadataError;
    let err = validate_timeseries(&metadata, "SELECT 1")
        .expect_err("refresh: incremental with neither shape fact must be a hard error");
    assert!(
        matches!(err, MetadataError::GrainRequiredForIncremental),
        "Expected GrainRequiredForIncremental, got: {}",
        err
    );
    let message = err.to_string();
    assert!(
        message.contains("timeseries") && message.contains("unique_key"),
        "error must name the missing shape-defining facts; got: {message}"
    );
}

/// A written `grain:` is a check-only assertion, never a driver: it must
/// agree with the label derived from the declared shape facts, or it is a
/// hard error naming both labels.
#[test]
fn grain_assertion_is_check_only() {
    use smelt_core::config::Grain;
    use smelt_core::metadata::MetadataError;

    // `grain: partition` on a model whose facts derive `key` (a declared
    // unique_key, no clock) → mismatch, names both labels.
    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Partition),
        unique_key: Some(vec!["order_id".to_string()]),
        ..Default::default()
    };
    let err = validate_timeseries(&metadata, "SELECT order_id FROM foo")
        .expect_err("grain: partition contradicting a facts-derived key shape must error");
    match &err {
        MetadataError::GrainAssertionMismatch { asserted, derived } => {
            assert_eq!(*asserted, Grain::Partition);
            assert_eq!(*derived, Grain::Key);
        }
        other => panic!("Expected GrainAssertionMismatch, got: {other}"),
    }
    let message = err.to_string();
    assert!(
        message.contains("partition") && message.contains("key"),
        "error must name both the asserted and derived grains; got: {message}"
    );

    // `grain: key` on a facts-derived key shape passes.
    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::Key),
        unique_key: Some(vec!["order_id".to_string()]),
        ..Default::default()
    };
    validate_timeseries(&metadata, "SELECT order_id FROM foo")
        .expect("grain: key agreeing with the facts-derived key shape must pass");

    // clock + identity + `partition_column ∈ key` derives `key_per_partition`
    // — the derivation agrees with the written assertion at the frontmatter
    // level (still refused later, at plan derivation, by A0's fail-loud
    // `key_per_partition` guard — a separate, composing diagnostic, not this
    // one; `docs/plans/20260715-composed-axes-conditional-maintenance.md`
    // Phase A0).
    use smelt_core::config::{Granularity, TimeseriesConfig};
    let metadata = ModelMetadata {
        materialization: Some(Materialization::Table),
        refresh: Some(RefreshStrategy::Incremental),
        grain: Some(Grain::KeyPerPartition),
        unique_key: Some(vec!["order_id".to_string(), "dt".to_string()]),
        timeseries: Some(TimeseriesConfig {
            event_time_column: "ts".to_string(),
            partition_column: "dt".to_string(),
            granularity: Granularity::Day,
            week_start: None,
            assert_monotonic: false,
        }),
        ..Default::default()
    };
    validate_timeseries(&metadata, "SELECT order_id, dt FROM foo").expect(
        "grain: key_per_partition agreeing with a partition_column ∈ key derivation must pass",
    );
    assert_eq!(metadata.resolved_grain(), Some(Grain::KeyPerPartition));
}
