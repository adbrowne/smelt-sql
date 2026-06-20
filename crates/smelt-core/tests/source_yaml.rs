//! TDD tests for Phase 6: per-entity source YAML loading.
//!
//! These tests are written FIRST and expected to FAIL until the implementation
//! is complete. Each test corresponds to a spec invariant from sources.md.

use smelt_core::resolver::WorkspaceLoadError;
use smelt_core::sources::{
    discover_source_infos, parse_source_yaml, SourceError, SourceNameOverride,
};
use std::fs;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helper: build a minimal project layout
// ---------------------------------------------------------------------------

fn make_project(tmp: &TempDir, paths: &[&str]) -> std::path::PathBuf {
    let root = tmp.path().to_path_buf();
    // Create smelt.yml with the given paths
    let paths_yaml: String = paths
        .iter()
        .map(|p| format!("  - {}\n", p))
        .collect::<String>();
    let smelt_yml = format!("name: test\npaths:\n{}", paths_yaml);
    fs::write(root.join("smelt.yml"), smelt_yml).unwrap();
    root
}

// ---------------------------------------------------------------------------
// Test 1: standalone_yml_loads_as_source
// ---------------------------------------------------------------------------

/// A `.yml` with no sibling `.csv` is a source.
/// `models/external/api/orders.yml` under `paths: ["models"]` →
/// `address_segments = ["external", "api", "orders"]`.
#[test]
fn standalone_yml_loads_as_source() {
    let tmp = TempDir::new().unwrap();
    let root = make_project(&tmp, &["models"]);

    let dir = root.join("models/external/api");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("orders.yml"),
        r#"
description: Orders from external API
columns:
  - name: order_id
    type: INTEGER
    nullable: false
  - name: customer_id
    type: INTEGER
  - name: placed_at
    type: TIMESTAMP
"#,
    )
    .unwrap();

    let sources = discover_source_infos(&root, &["models".to_string()]);
    assert_eq!(
        sources.len(),
        1,
        "expected exactly one source, got {sources:?}"
    );

    let src = &sources[0];
    assert_eq!(
        src.address_segments,
        vec!["external", "api", "orders"],
        "wrong address segments"
    );
    assert_eq!(src.columns.len(), 3);
    assert_eq!(src.columns[0].name, "order_id");
    assert!(!src.columns[0].nullable);
    assert!(src.description.is_some());
}

// ---------------------------------------------------------------------------
// Test 2: name_override_takes_precedence
// ---------------------------------------------------------------------------

/// `name: legacy_db.orders_v2` overrides the default DB name.
#[test]
fn name_override_takes_precedence() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("models");
    fs::create_dir_all(&dir).unwrap();

    fs::write(
        dir.join("orders.yml"),
        r#"
columns:
  - name: order_id
    type: INTEGER
name: legacy_db.orders_v2
"#,
    )
    .unwrap();

    let info = parse_source_yaml(&dir.join("orders.yml")).unwrap();
    assert!(
        matches!(
            &info.name_override,
            Some(SourceNameOverride::Literal(s)) if s == "legacy_db.orders_v2"
        ),
        "name_override should be Literal(\"legacy_db.orders_v2\"), got {:?}",
        info.name_override
    );

    // db_name should use the override, not the default mapping
    let db_name = info.db_name("main");
    assert_eq!(
        db_name, "legacy_db.orders_v2",
        "db_name() should return the Literal override verbatim"
    );
}

// ---------------------------------------------------------------------------
// Test 3: materialization_on_source_is_error
// ---------------------------------------------------------------------------

/// `materialization: table` on a source YAML is a hard parse error.
#[test]
fn materialization_on_source_is_error() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("models");
    fs::create_dir_all(&dir).unwrap();

    fs::write(
        dir.join("bad_source.yml"),
        r#"
materialization: table
columns:
  - name: id
    type: INTEGER
"#,
    )
    .unwrap();

    let err = parse_source_yaml(&dir.join("bad_source.yml")).unwrap_err();
    assert!(
        matches!(err, SourceError::MaterializationForbidden),
        "expected MaterializationForbidden, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 4: sidecar_takes_priority_over_source
// ---------------------------------------------------------------------------

/// A `.yml` next to a same-stem `.csv` is a sidecar, not a source.
/// `discover_source_infos` must NOT return it as a source.
#[test]
fn sidecar_takes_priority_over_source() {
    let tmp = TempDir::new().unwrap();
    let root = make_project(&tmp, &["models"]);

    let dir = root.join("models/seeds");
    fs::create_dir_all(&dir).unwrap();

    // Both CSV and YML exist → YML is a sidecar, not a source.
    fs::write(dir.join("customers.csv"), "id,name\n1,Alice\n").unwrap();
    fs::write(
        dir.join("customers.yml"),
        r#"
columns:
  - name: id
    type: INTEGER
"#,
    )
    .unwrap();

    let sources = discover_source_infos(&root, &["models".to_string()]);
    assert!(
        sources.is_empty(),
        "sidecar YAML should not be returned as a source, got: {sources:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 5: aggregate_sources_yml_errors
// ---------------------------------------------------------------------------

/// A project-root `sources.yml` should produce a hard
/// `WorkspaceLoadError::AggregateSourcesYmlNotSupported` when attempted to
/// be detected/loaded.
#[test]
fn aggregate_sources_yml_errors() {
    let tmp = TempDir::new().unwrap();
    let root = make_project(&tmp, &["models"]);

    // Place an aggregate sources.yml at the project root
    fs::write(
        root.join("sources.yml"),
        r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
"#,
    )
    .unwrap();

    // The detection function should flag this as an error
    let result = smelt_core::sources::check_aggregate_sources_yml(&root);
    assert!(
        result.is_err(),
        "aggregate sources.yml at project root should be an error"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(
            err,
            WorkspaceLoadError::AggregateSourcesYmlNotSupported { .. }
        ),
        "expected AggregateSourcesYmlNotSupported, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 6: name_override_validates_schema_table_format
// ---------------------------------------------------------------------------

/// `name:` must be in `<schema>.<table>` format.
#[test]
fn name_override_validates_schema_table_format() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("models");
    fs::create_dir_all(&dir).unwrap();

    // Single word (no dot) is invalid
    fs::write(
        dir.join("bad.yml"),
        r#"
columns:
  - name: id
    type: INTEGER
name: nodot
"#,
    )
    .unwrap();

    let err = parse_source_yaml(&dir.join("bad.yml")).unwrap_err();
    assert!(
        matches!(err, SourceError::InvalidNameOverride(_)),
        "expected InvalidNameOverride for single-word name, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 7: columns_required_on_source
// ---------------------------------------------------------------------------

/// `columns:` is required on a source YAML (unlike sidecar where it's optional).
#[test]
fn columns_required_on_source() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("models");
    fs::create_dir_all(&dir).unwrap();

    fs::write(
        dir.join("no_cols.yml"),
        "description: A source with no columns\n",
    )
    .unwrap();

    let err = parse_source_yaml(&dir.join("no_cols.yml")).unwrap_err();
    assert!(
        matches!(err, SourceError::MissingColumns),
        "expected MissingColumns, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 8: default_db_name_mapping
// ---------------------------------------------------------------------------

/// When no `name:` override, db_name() uses `<target_schema>.<segs.join("_")>`.
#[test]
fn default_db_name_mapping() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("models");
    fs::create_dir_all(&dir).unwrap();

    fs::write(
        dir.join("orders.yml"),
        r#"
columns:
  - name: id
    type: INTEGER
"#,
    )
    .unwrap();

    let info = parse_source_yaml(&dir.join("orders.yml")).unwrap();
    // address_segments set by parse_source_yaml based on path; but since we
    // call it directly, segments come from the file name only.
    // db_name uses the address_segments + target_schema.
    // For a top-level parse (no scan root), segments = ["orders"].
    let db_name = info.db_name("main");
    assert_eq!(db_name, "main.orders");
}

// ---------------------------------------------------------------------------
// D-35 P1: SourceNameOverride enum — per-target map form
// ---------------------------------------------------------------------------

/// D-35 P1: `name: { dev: raw_dev.users, prod: raw.users }` parses to PerTarget map.
#[test]
fn per_target_map_parses() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("models");
    fs::create_dir_all(&dir).unwrap();

    fs::write(
        dir.join("users.yml"),
        r#"
columns:
  - name: id
    type: INTEGER
name:
  dev: raw_dev.users
  prod: raw.users
"#,
    )
    .unwrap();

    let info = parse_source_yaml(&dir.join("users.yml")).unwrap();
    match &info.name_override {
        Some(SourceNameOverride::PerTarget(map)) => {
            assert_eq!(map.get("dev").map(String::as_str), Some("raw_dev.users"));
            assert_eq!(map.get("prod").map(String::as_str), Some("raw.users"));
        }
        other => panic!("expected PerTarget, got {other:?}"),
    }
}

/// D-35 P1: a per-target map value without a dot is InvalidNameOverride.
#[test]
fn per_target_map_value_invalid_format_is_error() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("models");
    fs::create_dir_all(&dir).unwrap();

    fs::write(
        dir.join("users.yml"),
        r#"
columns:
  - name: id
    type: INTEGER
name:
  dev: nodot
"#,
    )
    .unwrap();

    let err = parse_source_yaml(&dir.join("users.yml")).unwrap_err();
    assert!(
        matches!(err, SourceError::InvalidNameOverride(_)),
        "expected InvalidNameOverride for map value without dot, got {err:?}"
    );
}

/// D-35 P1: `name: raw_cdc.users` still parses as Literal (regression guard).
#[test]
fn literal_still_parses() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("models");
    fs::create_dir_all(&dir).unwrap();

    fs::write(
        dir.join("users.yml"),
        r#"
columns:
  - name: id
    type: INTEGER
name: raw_cdc.users
"#,
    )
    .unwrap();

    let info = parse_source_yaml(&dir.join("users.yml")).unwrap();
    match &info.name_override {
        Some(SourceNameOverride::Literal(s)) => assert_eq!(s, "raw_cdc.users"),
        other => panic!("expected Literal, got {other:?}"),
    }
}
