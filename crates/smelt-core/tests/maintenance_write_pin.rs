//! Tests for `maintenance.cells[].write` — the per-cell physical-addressing
//! pin (`docs/specs/incremental_models.md` §"Per-cell write addressing" →
//! "User pins", "The write-pattern set is open").
//!
//! This is a pure frontmatter-parsing surface: `write:` is an **open
//! string**, not a sealed enum — resolution against the write-pattern
//! registry (`smelt_logical::maintenance::lookup_write_pattern`) and the
//! two refusal diagnostics happen downstream (`smelt-db`), never here.

use smelt_core::config::{MaintenanceCellConfig, MaintenanceConfig, RefreshStrategy};
use smelt_core::metadata::{extract_file_metadata, FileMetadata};

/// `maintenance.cells[].write: <name>` parses as a plain string on
/// `MaintenanceCellConfig`, accepting any name — including one the registry
/// does not (yet) recognise. Validation against the registry is a
/// downstream (`smelt-db`) concern, not a parse-time one.
#[test]
fn write_pin_parses_as_an_open_string() {
    let source = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
maintenance:
  cells:
    - columns: [amount]
      on: backfill
      technique: recompute
      write: region
---
SELECT d, amount FROM smelt.sources.payments"#;

    let result = extract_file_metadata(source).expect("well-formed frontmatter must parse");
    match result {
        FileMetadata::Single { metadata, .. } => {
            assert_eq!(metadata.refresh, Some(RefreshStrategy::Incremental));
            let maintenance = metadata.maintenance.expect("maintenance block parsed");
            assert_eq!(maintenance.cells.len(), 1);
            assert_eq!(maintenance.cells[0].write.as_deref(), Some("region"));
        }
        _ => panic!("expected Single variant"),
    }
}

/// A name entirely unknown to the registry today (`not_a_real_pattern`)
/// still parses cleanly — the open-registry design's whole point: a new
/// backend-contributed pattern name is admitted without a `smelt-core`
/// release, and an unrecognised name is refused downstream
/// (`MaintenanceWritePatternUnavailable`), never at parse time.
#[test]
fn unrecognised_write_pin_name_still_parses_cleanly() {
    let source = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
maintenance:
  cells:
    - columns: [amount]
      on: backfill
      write: some_future_backend_pattern
---
SELECT d, amount FROM smelt.sources.payments"#;

    let result = extract_file_metadata(source).expect("parsing never validates the pin name");
    match result {
        FileMetadata::Single { metadata, .. } => {
            let maintenance = metadata.maintenance.expect("maintenance block parsed");
            assert_eq!(
                maintenance.cells[0].write.as_deref(),
                Some("some_future_backend_pattern")
            );
        }
        _ => panic!("expected Single variant"),
    }
}

/// A cell with no `write:` key leaves the field `None` — the pin is
/// optional; absent it, the derived plan/cost model chooses the addressing
/// as it always has.
#[test]
fn absent_write_key_is_none() {
    let source = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
maintenance:
  cells:
    - columns: [amount]
      on: backfill
---
SELECT d, amount FROM smelt.sources.payments"#;

    let result = extract_file_metadata(source).expect("well-formed frontmatter must parse");
    match result {
        FileMetadata::Single { metadata, .. } => {
            let maintenance = metadata.maintenance.expect("maintenance block parsed");
            assert_eq!(maintenance.cells[0].write, None);
        }
        _ => panic!("expected Single variant"),
    }
}

/// A malformed `cells:` entry — `write:` given a non-scalar (a mapping)
/// value — fails `MaintenanceConfig`'s own `serde` deserialization with a
/// YAML parse error, the same failure `smelt-core::metadata::MetadataError::
/// YamlParseError` wraps and the same boundary `smelt-db`'s frontmatter
/// diagnostics deserialize check reuses (`smelt-db/src/lib.rs`'s "nested
/// sub-field failure" catch). Exercised directly against
/// `MaintenanceConfig`'s deserializer rather than through
/// `extract_file_metadata` — that entry point's resilient fallback (`smelt-
/// core/src/metadata.rs::extract_single_model`) deliberately swallows a
/// malformed nested block into `ModelMetadata::default()` so the model stays
/// discoverable while `smelt-db` raises the real diagnostic; this test
/// asserts the underlying parse genuinely fails rather than silently
/// coercing `write:` to a string.
#[test]
fn malformed_write_pin_value_is_a_metadata_error() {
    let yaml = r#"
cells:
  - columns: [amount]
    on: backfill
    write:
      nested: not_a_string
"#;
    let err = serde_yaml::from_str::<MaintenanceConfig>(yaml)
        .expect_err("a mapping value where a string is expected must fail to parse");
    let metadata_err = smelt_core::metadata::MetadataError::YamlParseError(err);
    assert!(matches!(
        metadata_err,
        smelt_core::metadata::MetadataError::YamlParseError(_)
    ));
}

/// A malformed `cells:` entry — an unknown field alongside `write:` — also
/// fails to deserialize, per `MaintenanceCellConfig`'s `deny_unknown_fields`.
/// Same direct-deserializer boundary as the test above.
#[test]
fn cells_entry_with_unknown_field_is_a_metadata_error() {
    let yaml = r#"
cells:
  - columns: [amount]
    on: backfill
    write: region
    not_a_real_field: true
"#;
    let err = serde_yaml::from_str::<MaintenanceConfig>(yaml)
        .expect_err("an unknown cells[] field must fail to parse (deny_unknown_fields)");
    let metadata_err = smelt_core::metadata::MetadataError::YamlParseError(err);
    assert!(matches!(
        metadata_err,
        smelt_core::metadata::MetadataError::YamlParseError(_)
    ));
}

/// Constructing `MaintenanceCellConfig` directly (the type callers outside
/// YAML parsing build, e.g. `smelt-logical`'s tests) confirms `write` is a
/// plain `Option<String>` field, not an enum — any string compiles.
#[test]
fn maintenance_cell_config_write_field_is_plain_option_string() {
    let cell = MaintenanceCellConfig {
        columns: vec!["amount".to_string()],
        on: "backfill".to_string(),
        prefer: None,
        technique: None,
        write: Some("anything_at_all".to_string()),
    };
    assert_eq!(cell.write.as_deref(), Some("anything_at_all"));
}
