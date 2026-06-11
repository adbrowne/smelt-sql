//! Real-fixture integration tests for outer-join nullability on `smelt.sources.*` references.
//!
//! These tests drive the full Salsa pipeline (`typed_model_schema`) rather than
//! the synthetic `TypeContext` used in `nullability_property_tests.rs`.  They
//! exercise:
//!   - `entity_name_for_table_ref` on a real `smelt.sources.<schema>.<table>` path-ref with alias
//!   - alias → entity → source-key resolution through the Salsa `type_context` query
//!   - `mark_entity_columns_nullable`'s `source_columns` branch on the real path
//!
//! Fixture shape:
//!   - `smelt.yml`
//!   - `models/sources/raw/events.yml`  (columns declared `nullable: false`)
//!   - `models/<model>.sql`  (LEFT / RIGHT / FULL / INNER JOIN over the source)

use std::fs;

use smelt_db::{typed_model_schema, Database, Workspace};
use smelt_types::DataType;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Shared fixture helpers
// ---------------------------------------------------------------------------

const SMELT_YML: &str = "name: outer_join_nullability_fixture\n\
version: 1\n\
paths:\n  - models\n\
targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
default_materialization: view\n";

/// Source YAML with two columns both declared `nullable: false`.
const EVENTS_SOURCE_YAML: &str = "description: Events source for outer-join nullability test\n\
columns:\n\
- name: event_id\n  type: INTEGER\n  nullable: false\n\
- name: user_id\n  type: INTEGER\n  nullable: false\n\
- name: event_type\n  type: TEXT\n  nullable: false\n";

/// Source YAML with a users table, also non-nullable.
const USERS_SOURCE_YAML: &str = "description: Users source for outer-join nullability test\n\
columns:\n\
- name: user_id\n  type: INTEGER\n  nullable: false\n\
- name: name\n  type: TEXT\n  nullable: false\n";

fn stage_files(files: &[(&str, &str)]) -> TempDir {
    let tmp = TempDir::new().expect("create tempdir");
    for (rel, contents) in files {
        let path = tmp.path().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create_dir_all");
        }
        fs::write(&path, contents).expect("write file");
    }
    tmp
}

/// Ingest a project rooted at `tmp.path()` with the given SQL model files.
/// Returns `(Database, Workspace, SourceFile)` for the model at `target_rel`.
fn ingest(
    tmp: &TempDir,
    model_files: &[(&str, &str)],
    target_rel: &str,
) -> (Database, Workspace, smelt_db::SourceFile) {
    let project_root = tmp.path().to_path_buf();
    let mut db = Database::default();

    let project = db.set_project_input(project_root.clone(), String::new());

    let mut source_files = Vec::new();
    for (rel, content) in model_files {
        let path = project_root.join(rel);
        let sf = db.set_source_file(path, content.to_string(), project_root.clone());
        source_files.push(sf);
    }
    db.set_workspace(source_files.clone(), vec![project]);
    let ws = db.workspace();

    let target_path = project_root.join(target_rel);
    let target_sf = *source_files
        .iter()
        .find(|sf| sf.path(&db) == &target_path)
        .expect("target file not found in registered files");

    (db, ws, target_sf)
}

// ---------------------------------------------------------------------------
// Test 1: LEFT JOIN — right-side source columns become nullable
// ---------------------------------------------------------------------------

/// Real-fixture test: a model that LEFT JOINs a `smelt.sources.raw.events`
/// source declared `nullable: false` must infer `nullable: true` for the
/// right-side (events) columns in the output schema.
///
/// This is the core regression guard for the `source_columns` branch of
/// `mark_entity_columns_nullable`.  The synthetic test in
/// `nullability_property_tests.rs` uses `add_model_column` (not
/// `add_source_column`), so it does not exercise the source-column key path.
#[test]
fn left_join_source_nullable_false_becomes_nullable_true() {
    // Model: users LEFT JOIN events on user_id.
    // events columns are declared nullable: false in the source YAML.
    // After a LEFT JOIN, the right side (events) must be forced nullable.
    let model_sql = "\
SELECT \
  u.user_id AS user_id, \
  e.event_id AS event_id, \
  e.event_type AS event_type \
FROM smelt.sources.raw.users AS u \
LEFT JOIN smelt.sources.raw.events AS e ON u.user_id = e.user_id\n";

    let tmp = stage_files(&[
        ("smelt.yml", SMELT_YML),
        ("models/join_model.sql", model_sql),
        ("models/sources/raw/events.yml", EVENTS_SOURCE_YAML),
        ("models/sources/raw/users.yml", USERS_SOURCE_YAML),
    ]);

    let (db, ws, model_file) = ingest(
        &tmp,
        &[("models/join_model.sql", model_sql)],
        "models/join_model.sql",
    );

    let schema = typed_model_schema(&db, ws, model_file);

    // Build a map: column name → TypedColumn
    let col_map: std::collections::HashMap<_, _> = schema
        .columns
        .iter()
        .filter_map(|c| c.data_type.as_ref().map(|dt| (c.name.as_str(), dt.clone())))
        .collect();

    assert!(
        !col_map.is_empty(),
        "typed_model_schema returned no typed columns — source columns were not loaded. \
         Schema: {:#?}",
        schema.columns
    );

    // Left side (users) — NOT nullable-forced (they're on the preserved side of LEFT JOIN).
    // user_id was nullable: false in the source; LEFT JOIN left side stays as-is.
    let user_id = col_map.get("user_id").expect("user_id column not found");
    assert_eq!(
        user_id.data_type,
        DataType::Integer,
        "user_id should have data type INTEGER"
    );
    assert!(
        !user_id.nullable,
        "user_id is on the LEFT (preserved) side of a LEFT JOIN — \
         it should remain non-nullable (nullable: false). Got nullable: {}",
        user_id.nullable
    );

    // Right side (events) — MUST be nullable after LEFT JOIN.
    let event_id = col_map.get("event_id").expect("event_id column not found");
    assert_eq!(
        event_id.data_type,
        DataType::Integer,
        "event_id should have data type INTEGER"
    );
    assert!(
        event_id.nullable,
        "event_id is declared nullable: false in the source YAML, but appears on the RIGHT \
         side of a LEFT JOIN (the null-supplying side) — apply_outer_join_nullability must \
         mark it nullable: true via the source_columns path. \
         Got nullable: {} — this is the source-column key bug: mark_entity_columns_nullable \
         may not be flipping the correct source_columns key form.",
        event_id.nullable
    );

    let event_type = col_map
        .get("event_type")
        .expect("event_type column not found");
    assert!(
        event_type.nullable,
        "event_type is declared nullable: false in the source YAML, but appears on the RIGHT \
         side of a LEFT JOIN — must be forced nullable: true. Got nullable: {}",
        event_type.nullable
    );
}

// ---------------------------------------------------------------------------
// Test 2: INNER JOIN — source nullable: false must NOT be overridden
// ---------------------------------------------------------------------------

/// Real-fixture INNER JOIN: columns from a source declared `nullable: false`
/// must remain non-nullable after an INNER JOIN (no null-supplying side).
///
/// This guards against `mark_entity_columns_nullable` over-flipping via the
/// source path — an INNER JOIN must not change nullability.
#[test]
fn inner_join_source_nullable_false_stays_non_nullable() {
    let model_sql = "\
SELECT \
  u.user_id AS user_id, \
  e.event_id AS event_id, \
  e.event_type AS event_type \
FROM smelt.sources.raw.users AS u \
INNER JOIN smelt.sources.raw.events AS e ON u.user_id = e.user_id\n";

    let tmp = stage_files(&[
        ("smelt.yml", SMELT_YML),
        ("models/inner_join_model.sql", model_sql),
        ("models/sources/raw/events.yml", EVENTS_SOURCE_YAML),
        ("models/sources/raw/users.yml", USERS_SOURCE_YAML),
    ]);

    let (db, ws, model_file) = ingest(
        &tmp,
        &[("models/inner_join_model.sql", model_sql)],
        "models/inner_join_model.sql",
    );

    let schema = typed_model_schema(&db, ws, model_file);

    let col_map: std::collections::HashMap<_, _> = schema
        .columns
        .iter()
        .filter_map(|c| c.data_type.as_ref().map(|dt| (c.name.as_str(), dt.clone())))
        .collect();

    assert!(
        !col_map.is_empty(),
        "typed_model_schema returned no typed columns for INNER JOIN model. \
         Schema: {:#?}",
        schema.columns
    );

    // Both sides of INNER JOIN — should stay nullable: false (as declared in source).
    let user_id = col_map.get("user_id").expect("user_id column not found");
    assert!(
        !user_id.nullable,
        "user_id is declared nullable: false in the source YAML and appears in an INNER JOIN — \
         nullability must NOT be forced. Got nullable: {}",
        user_id.nullable
    );

    let event_id = col_map.get("event_id").expect("event_id column not found");
    assert!(
        !event_id.nullable,
        "event_id is declared nullable: false in the source YAML and appears in an INNER JOIN — \
         nullability must NOT be forced. Got nullable: {}",
        event_id.nullable
    );

    let event_type = col_map
        .get("event_type")
        .expect("event_type column not found");
    assert!(
        !event_type.nullable,
        "event_type is declared nullable: false in the source YAML and appears in an INNER JOIN — \
         nullability must NOT be forced. Got nullable: {}",
        event_type.nullable
    );
}

// ---------------------------------------------------------------------------
// Test 3: RIGHT JOIN — left-side source columns become nullable
// ---------------------------------------------------------------------------

/// Real-fixture RIGHT JOIN: the left-side source (users) columns must be
/// forced nullable; the right-side (events) columns stay as declared.
#[test]
fn right_join_left_source_becomes_nullable() {
    let model_sql = "\
SELECT \
  u.user_id AS user_id, \
  e.event_id AS event_id \
FROM smelt.sources.raw.users AS u \
RIGHT JOIN smelt.sources.raw.events AS e ON u.user_id = e.user_id\n";

    let tmp = stage_files(&[
        ("smelt.yml", SMELT_YML),
        ("models/right_join_model.sql", model_sql),
        ("models/sources/raw/events.yml", EVENTS_SOURCE_YAML),
        ("models/sources/raw/users.yml", USERS_SOURCE_YAML),
    ]);

    let (db, ws, model_file) = ingest(
        &tmp,
        &[("models/right_join_model.sql", model_sql)],
        "models/right_join_model.sql",
    );

    let schema = typed_model_schema(&db, ws, model_file);

    let col_map: std::collections::HashMap<_, _> = schema
        .columns
        .iter()
        .filter_map(|c| c.data_type.as_ref().map(|dt| (c.name.as_str(), dt.clone())))
        .collect();

    assert!(
        !col_map.is_empty(),
        "typed_model_schema returned no typed columns for RIGHT JOIN model. \
         Schema: {:#?}",
        schema.columns
    );

    // Left side (users) — nullable-forced by RIGHT JOIN.
    let user_id = col_map.get("user_id").expect("user_id column not found");
    assert!(
        user_id.nullable,
        "user_id is on the LEFT side of a RIGHT JOIN (null-supplying side) — \
         must be forced nullable: true. Got nullable: {}",
        user_id.nullable
    );

    // Right side (events) — stays non-nullable (preserved side of RIGHT JOIN).
    let event_id = col_map.get("event_id").expect("event_id column not found");
    assert!(
        !event_id.nullable,
        "event_id is on the RIGHT (preserved) side of a RIGHT JOIN — \
         must remain nullable: false as declared. Got nullable: {}",
        event_id.nullable
    );
}

// ---------------------------------------------------------------------------
// Test 4: FULL JOIN — both sides become nullable
// ---------------------------------------------------------------------------

/// Real-fixture FULL JOIN: both sides must be forced nullable.
#[test]
fn full_join_both_sides_become_nullable() {
    let model_sql = "\
SELECT \
  u.user_id AS user_id, \
  e.event_id AS event_id \
FROM smelt.sources.raw.users AS u \
FULL JOIN smelt.sources.raw.events AS e ON u.user_id = e.user_id\n";

    let tmp = stage_files(&[
        ("smelt.yml", SMELT_YML),
        ("models/full_join_model.sql", model_sql),
        ("models/sources/raw/events.yml", EVENTS_SOURCE_YAML),
        ("models/sources/raw/users.yml", USERS_SOURCE_YAML),
    ]);

    let (db, ws, model_file) = ingest(
        &tmp,
        &[("models/full_join_model.sql", model_sql)],
        "models/full_join_model.sql",
    );

    let schema = typed_model_schema(&db, ws, model_file);

    let col_map: std::collections::HashMap<_, _> = schema
        .columns
        .iter()
        .filter_map(|c| c.data_type.as_ref().map(|dt| (c.name.as_str(), dt.clone())))
        .collect();

    assert!(
        !col_map.is_empty(),
        "typed_model_schema returned no typed columns for FULL JOIN model. \
         Schema: {:#?}",
        schema.columns
    );

    // Both sides must be nullable after FULL JOIN.
    let user_id = col_map.get("user_id").expect("user_id column not found");
    assert!(
        user_id.nullable,
        "user_id is in a FULL JOIN — both sides must be forced nullable: true. \
         Got nullable: {}",
        user_id.nullable
    );

    let event_id = col_map.get("event_id").expect("event_id column not found");
    assert!(
        event_id.nullable,
        "event_id is in a FULL JOIN — both sides must be forced nullable: true. \
         Got nullable: {}",
        event_id.nullable
    );
}
