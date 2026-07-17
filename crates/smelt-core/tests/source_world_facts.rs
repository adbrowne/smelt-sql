//! TDD tests for the structured `mutation_profile` block, `watermark:`,
//! composite `unique_key:`, and `retention:` (Phase MP3 of
//! `docs/plans/20260707-maintenance-plan-impl.md`).
//!
//! Spec: `docs/specs/sources.md` §"Source YAML shape",
//! §"`mutation_profile` — the structured block", §Semantics item 3 "The
//! trust rule", and the Diagnostic codes table.

use smelt_core::sources::{
    parse_source_yaml, MutationProfile, Redelivery, SourceError, SourceMutationProfile,
};
use std::fs;
use tempfile::TempDir;

fn write_source(dir: &TempDir, body: &str) -> std::path::PathBuf {
    let path = dir.path().join("events.yml");
    fs::write(&path, body).unwrap();
    path
}

// ---------------------------------------------------------------------------
// structured_block_parses_all_kinds
// ---------------------------------------------------------------------------

#[test]
fn structured_block_parses_all_kinds_append_only() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: conversion_id, type: BIGINT, nullable: false }
mutation_profile:
  kind: append_only
  lateness: '2 days'
  redelivery: none
"#,
    );

    let info = parse_source_yaml(&path).expect("should parse");
    let profile = info.mutation_profile.expect("mutation_profile is Some");
    assert_eq!(profile.kind, MutationProfile::AppendOnly);
    assert_eq!(profile.lateness.expect("lateness declared").to_days(), 2);
    assert_eq!(profile.redelivery, Redelivery::None);
    // change_feed sub-facts absent, default posture.
    assert!(profile.retractions, "retractions defaults to true");
    assert_eq!(profile.ordered, None);
    assert_eq!(profile.delta_identity, None);
}

#[test]
fn structured_block_parses_all_kinds_mutable_snapshot() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: id, type: BIGINT, nullable: false }
mutation_profile:
  kind: mutable_snapshot
"#,
    );

    let info = parse_source_yaml(&path).expect("should parse");
    let profile = info.mutation_profile.expect("mutation_profile is Some");
    assert_eq!(profile.kind, MutationProfile::Mutable);
    assert_eq!(profile.lateness, None);
    assert_eq!(profile.redelivery, Redelivery::AtLeastOnce);
}

#[test]
fn structured_block_parses_all_kinds_change_feed_with_key_recurrence() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: user_id, type: BIGINT, nullable: false }
mutation_profile:
  kind: change_feed
  retractions: false
  ordered: true
  delta_identity: [_commit_version, _row_offset]
  key_recurrence:
    key: [user_id]
    window: '3 days'
"#,
    );

    let info = parse_source_yaml(&path).expect("should parse");
    let profile = info.mutation_profile.expect("mutation_profile is Some");
    assert_eq!(profile.kind, MutationProfile::ChangeFeed);
    assert!(!profile.retractions);
    assert_eq!(profile.ordered, Some(true));
    assert_eq!(
        profile.delta_identity,
        Some(vec![
            "_commit_version".to_string(),
            "_row_offset".to_string()
        ])
    );
    let kr = profile.key_recurrence.expect("key_recurrence declared");
    assert_eq!(kr.key, vec!["user_id".to_string()]);
    assert_eq!(kr.window.to_days(), 3);
}

#[test]
fn bare_string_shorthand_normalizes_to_structured_block() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: id, type: BIGINT, nullable: false }
mutation_profile: append_only
"#,
    );

    let info = parse_source_yaml(&path).expect("should parse");
    let profile = info.mutation_profile.expect("mutation_profile is Some");
    assert_eq!(
        profile,
        SourceMutationProfile::from_kind(MutationProfile::AppendOnly)
    );
}

// ---------------------------------------------------------------------------
// lateness_alias_double_declare_errors
// ---------------------------------------------------------------------------

#[test]
fn lateness_alias_double_declare_errors() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: id, type: BIGINT, nullable: false }
mutation_profile:
  kind: append_only
  lateness: '1 day'
source_lateness: '2 hours'
"#,
    );

    let err = parse_source_yaml(&path).expect_err("double-declared lateness must be an error");
    assert!(
        matches!(err, SourceError::LatenessDoubleDeclared),
        "expected LatenessDoubleDeclared, got: {err:?}"
    );
}

#[test]
fn lateness_alias_populates_from_source_lateness_when_undeclared_in_block() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: id, type: BIGINT, nullable: false }
mutation_profile:
  kind: append_only
source_lateness: '5 hours'
"#,
    );

    let info = parse_source_yaml(&path).expect("should parse");
    let profile = info.mutation_profile.expect("mutation_profile is Some");
    assert_eq!(
        profile.lateness.expect("aliased lateness").seconds,
        5 * 3600
    );
}

#[test]
fn lateness_alias_works_with_bare_shorthand_and_non_append_only_kind() {
    // Real-fixture shape (examples/source_mutation_profile_declared): a bare
    // `mutation_profile: change_feed` plus top-level `source_lateness:` must
    // keep parsing — the alias route bypasses the wrong-kind sub-fact check
    // because it was never declared *inside* the block.
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: id, type: BIGINT, nullable: false }
mutation_profile: change_feed
source_lateness: '2 hours'
"#,
    );

    let info = parse_source_yaml(&path).expect("should parse");
    let profile = info.mutation_profile.expect("mutation_profile is Some");
    assert_eq!(profile.kind, MutationProfile::ChangeFeed);
    assert_eq!(
        profile.lateness.expect("aliased lateness").seconds,
        2 * 3600
    );
}

// ---------------------------------------------------------------------------
// unique_key_is_composite_valued
// ---------------------------------------------------------------------------

#[test]
fn unique_key_is_composite_valued() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: conversion_id, type: BIGINT, nullable: false }
unique_key: [conversion_id]
"#,
    );
    let info = parse_source_yaml(&path).expect("should parse");
    assert_eq!(info.unique_key, Some(vec!["conversion_id".to_string()]));

    // Single-string shorthand is sugar for a one-element list.
    let tmp2 = TempDir::new().unwrap();
    let path2 = write_source(
        &tmp2,
        r#"
columns:
  - { name: conversion_id, type: BIGINT, nullable: false }
unique_key: conversion_id
"#,
    );
    let info2 = parse_source_yaml(&path2).expect("should parse");
    assert_eq!(info2.unique_key, Some(vec!["conversion_id".to_string()]));
    assert_eq!(info.unique_key, info2.unique_key);
}

#[test]
fn unique_key_composite_list_round_trips() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: entity_id, type: BIGINT, nullable: false }
  - { name: valid_from, type: DATE, nullable: false }
unique_key: [entity_id, valid_from]
"#,
    );
    let info = parse_source_yaml(&path).expect("should parse");
    assert_eq!(
        info.unique_key,
        Some(vec!["entity_id".to_string(), "valid_from".to_string()])
    );
}

// ---------------------------------------------------------------------------
// watermark / retention
// ---------------------------------------------------------------------------

#[test]
fn watermark_and_retention_parse() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: conversion_id, type: BIGINT, nullable: false }
  - { name: conversion_ts, type: TIMESTAMP, nullable: false }
watermark:
  complete_through: conversion_ts
retention: '400 days'
"#,
    );
    let info = parse_source_yaml(&path).expect("should parse");
    assert_eq!(
        info.watermark.expect("watermark declared").complete_through,
        "conversion_ts"
    );
    assert_eq!(info.retention.expect("retention declared").to_days(), 400);
}

// ---------------------------------------------------------------------------
// wrong-kind sub-fact validation
// ---------------------------------------------------------------------------

#[test]
fn change_feed_sub_fact_on_append_only_is_malformed() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: id, type: BIGINT, nullable: false }
mutation_profile:
  kind: append_only
  retractions: false
"#,
    );
    let err = parse_source_yaml(&path).expect_err("retractions under append_only must error");
    assert!(
        matches!(
            err,
            SourceError::MutationProfileSubFactWrongKind {
                field: "retractions",
                ..
            }
        ),
        "expected MutationProfileSubFactWrongKind, got: {err:?}"
    );
}

#[test]
fn append_only_sub_fact_on_change_feed_is_malformed() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: id, type: BIGINT, nullable: false }
mutation_profile:
  kind: change_feed
  redelivery: none
"#,
    );
    let err = parse_source_yaml(&path).expect_err("redelivery under change_feed must error");
    assert!(
        matches!(
            err,
            SourceError::MutationProfileSubFactWrongKind {
                field: "redelivery",
                ..
            }
        ),
        "expected MutationProfileSubFactWrongKind, got: {err:?}"
    );
}

#[test]
fn append_only_sub_fact_on_mutable_snapshot_is_malformed() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: id, type: BIGINT, nullable: false }
mutation_profile:
  kind: mutable_snapshot
  lateness: '1 day'
"#,
    );
    let err = parse_source_yaml(&path).expect_err("lateness under mutable_snapshot must error");
    assert!(
        matches!(
            err,
            SourceError::MutationProfileSubFactWrongKind {
                field: "lateness",
                ..
            }
        ),
        "expected MutationProfileSubFactWrongKind, got: {err:?}"
    );
}

#[test]
fn unrecognised_mutation_profile_kind_is_malformed() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: id, type: BIGINT, nullable: false }
mutation_profile:
  kind: not_a_real_kind
"#,
    );
    assert!(
        parse_source_yaml(&path).is_err(),
        "an unrecognised mutation_profile.kind must be a fail-loud parse error"
    );
}

#[test]
fn key_recurrence_missing_window_is_malformed() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: user_id, type: BIGINT, nullable: false }
mutation_profile:
  kind: append_only
  key_recurrence:
    key: [user_id]
"#,
    );
    assert!(
        parse_source_yaml(&path).is_err(),
        "key_recurrence without window must be a fail-loud parse error"
    );
}

#[test]
fn key_recurrence_single_string_key_is_composite_valued() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: user_id, type: BIGINT, nullable: false }
mutation_profile:
  kind: append_only
  key_recurrence:
    key: user_id
    window: '3 days'
"#,
    );
    let info = parse_source_yaml(&path).expect("should parse");
    let profile = info.mutation_profile.expect("mutation_profile is Some");
    let kr = profile.key_recurrence.expect("key_recurrence declared");
    assert_eq!(kr.key, vec!["user_id".to_string()]);
}

#[test]
fn unknown_key_in_mutation_profile_block_is_loud() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: id, type: BIGINT, nullable: false }
mutation_profile:
  kind: append_only
  bogus_subfact: true
"#,
    );
    assert!(
        parse_source_yaml(&path).is_err(),
        "an unknown mutation_profile sub-fact key must be a fail-loud parse error"
    );
}

// ---------------------------------------------------------------------------
// source_derived_grain (Phase S2 of
// `docs/plans/20260715-composed-axes-conditional-maintenance.md`):
// `SourceInfo::resolved_grain` derives the Relation Contract `grain` label
// from a source's own clock/identity facts, via the same `derive_grain`
// pure function a model output's `resolved_grain` reads
// (`docs/specs/sources.md` §"The source as a Relation Contract provider").
// ---------------------------------------------------------------------------

#[test]
fn source_derived_grain_clocked_fact_no_identity() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: event_id, type: BIGINT, nullable: false }
  - { name: event_date, type: DATE, nullable: false }
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
"#,
    );
    let info = parse_source_yaml(&path).expect("should parse");
    assert_eq!(
        info.resolved_grain(),
        Some(smelt_core::config::Grain::Partition),
        "a clock with no declared identity derives the clocked-fact (partition) label"
    );
}

#[test]
fn source_derived_grain_keyed_dimension_no_clock() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: user_id, type: BIGINT, nullable: false }
  - { name: tier, type: VARCHAR, nullable: true }
unique_key: user_id
"#,
    );
    let info = parse_source_yaml(&path).expect("should parse");
    assert_eq!(
        info.resolved_grain(),
        Some(smelt_core::config::Grain::Key),
        "a declared identity with no clock derives the keyed-dimension (key) label"
    );
}

#[test]
fn source_derived_grain_trajectory_when_partition_column_in_key() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: user_id, type: BIGINT, nullable: false }
  - { name: event_date, type: DATE, nullable: false }
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
unique_key:
  - user_id
  - event_date
"#,
    );
    let info = parse_source_yaml(&path).expect("should parse");
    assert_eq!(
        info.resolved_grain(),
        Some(smelt_core::config::Grain::KeyPerPartition),
        "clock + identity with partition_column in the key derives the trajectory label"
    );
}

#[test]
fn source_derived_grain_keyed_with_fixed_home_slice_when_partition_column_not_in_key() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: user_id, type: BIGINT, nullable: false }
  - { name: event_date, type: DATE, nullable: false }
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
unique_key:
  - user_id
"#,
    );
    let info = parse_source_yaml(&path).expect("should parse");
    assert_eq!(
        info.resolved_grain(),
        Some(smelt_core::config::Grain::Key),
        "clock + identity with partition_column NOT in the key still derives key \
         (time-partitioned lookup), not the trajectory"
    );
}

#[test]
fn source_derived_grain_unclassified_when_neither_fact_declared() {
    let tmp = TempDir::new().unwrap();
    let path = write_source(
        &tmp,
        r#"
columns:
  - { name: id, type: BIGINT, nullable: false }
"#,
    );
    let info = parse_source_yaml(&path).expect("should parse");
    assert_eq!(
        info.resolved_grain(),
        None,
        "a source declaring neither clock nor identity is legal — reported as \
         unclassified, never refused"
    );
}
