//! Tests for `smelt.sources.<path>` resolving under the sources namespace even
//! when the leaf segment collides with a model name in the same project.
//!
//! Tests:
//!   1. `source_leaf_collision_via_model_function_type` — the CLI (`smelt type`)
//!      path: a project with `models/orders.sql` (FROM smelt.sources.raw.orders)
//!      and `models/sources/raw/orders.yml` (id: INTEGER, amount: DOUBLE) must
//!      produce concrete types INTEGER/DOUBLE, not UNKNOWN. Drives the real
//!      `model_function_type` Salsa query.
//!   2. `source_leaf_collides_with_model_name` — same setup but driven via
//!      `typed_model_schema`. Confirms both query paths see the same fix.
//!   3. `source_no_collision_still_typed` — when the consuming model name does
//!      NOT collide with the source leaf, the typed path still works (regression
//!      guard against the fix breaking the non-collision path).

use std::fs;

use smelt_db::{typed_model_schema, Database, Workspace};
use smelt_types::DataType;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

const SMELT_YML: &str = "name: collision_fixture\n\
version: 1\n\
paths:\n  - models\n\
targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
default_materialization: view\n";

const SOURCE_YAML: &str = "description: Raw orders for leaf-collision test\n\
columns:\n\
- name: id\n  type: INTEGER\n\
- name: amount\n  type: DOUBLE\n";

/// Stage `files` under a new tempdir and return the tempdir guard.
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

/// Ingest a project rooted at `tmp.path()` with the given SQL model files into
/// a Salsa database.  Returns `(Database, Workspace, SourceFile)` for the
/// model file at `target_rel` (relative to `tmp.path()`).
///
/// Per-entity source YAMLs are discovered automatically from disk by the
/// `project_sources` Salsa query (keyed on `ProjectInput`).  No legacy
/// `sources.yml` content is needed.
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
// Test 1: model_function_type (the CLI path) — leaf collision case
// ---------------------------------------------------------------------------

/// The `smelt type` CLI command uses `model_function_type`. When `models/orders.sql`
/// reads `FROM smelt.sources.raw.orders` and `models/sources/raw/orders.yml`
/// declares `id: INTEGER, amount: DOUBLE`, the output schema must show
/// INTEGER and DOUBLE — not UNKNOWN — even though the model and source leaf
/// share the name `orders`.
///
/// Before the fix, calling `model_input_constraints` first (inside
/// `model_function_type`) would cache a `TypeContext` where the source columns
/// were shadowed by Unknown model columns written during the Salsa cycle
/// resolution of `resolved_columns("orders")`. That cached context was then
/// reused by `typed_model_schema`, causing every output column to show Unknown.
#[test]
fn source_leaf_collision_via_model_function_type() {
    let model_sql = "SELECT id, amount FROM smelt.sources.raw.orders\n";

    let tmp = stage_files(&[
        ("smelt.yml", SMELT_YML),
        ("models/orders.sql", model_sql),
        ("models/sources/raw/orders.yml", SOURCE_YAML),
    ]);

    let (db, ws, orders_file) = ingest(
        &tmp,
        &[("models/orders.sql", model_sql)],
        "models/orders.sql",
    );

    let ft = smelt_db::model_function_type(&db, ws, orders_file);

    let output_types: std::collections::HashMap<_, _> = ft
        .outputs
        .iter()
        .filter_map(|o| {
            o.data_type
                .as_ref()
                .map(|dt| (o.name.as_str(), dt.data_type.clone()))
        })
        .collect();

    assert_eq!(
        output_types.get("id"),
        Some(&DataType::Integer),
        "model_function_type: expected `id` = Integer, got {:?}\nFull ft: {}",
        output_types.get("id"),
        ft
    );

    assert_eq!(
        output_types.get("amount"),
        Some(&DataType::Double),
        "model_function_type: expected `amount` = Double, got {:?}\nFull ft: {}",
        output_types.get("amount"),
        ft
    );
}

// ---------------------------------------------------------------------------
// Test 2: typed_model_schema — leaf collision case
// ---------------------------------------------------------------------------

/// Companion to `source_leaf_collision_via_model_function_type`, driving the
/// `typed_model_schema` query directly instead of through `model_function_type`.
/// Both query paths must see concrete types from the source YAML.
#[test]
fn source_leaf_collides_with_model_name() {
    let model_sql = "SELECT id, amount FROM smelt.sources.raw.orders\n";

    let tmp = stage_files(&[
        ("smelt.yml", SMELT_YML),
        ("models/orders.sql", model_sql),
        ("models/sources/raw/orders.yml", SOURCE_YAML),
    ]);

    let (db, ws, orders_file) = ingest(
        &tmp,
        &[("models/orders.sql", model_sql)],
        "models/orders.sql",
    );

    let schema = typed_model_schema(&db, ws, orders_file);

    let col_map: std::collections::HashMap<_, _> = schema
        .columns
        .iter()
        .filter_map(|c| {
            c.data_type
                .as_ref()
                .map(|dt| (c.name.as_str(), dt.data_type.clone()))
        })
        .collect();

    assert!(
        !col_map.is_empty(),
        "typed_model_schema returned no typed columns; source columns were not loaded"
    );

    assert_eq!(
        col_map.get("id"),
        Some(&DataType::Integer),
        "expected `id` to be Integer (from source YAML), got {:?}\nFull schema: {:#?}",
        col_map.get("id"),
        schema.columns
    );

    assert_eq!(
        col_map.get("amount"),
        Some(&DataType::Double),
        "expected `amount` to be Double (from source YAML), got {:?}\nFull schema: {:#?}",
        col_map.get("amount"),
        schema.columns
    );
}

// ---------------------------------------------------------------------------
// Test 3: regression — no-collision path still resolves correctly
// ---------------------------------------------------------------------------

/// When the consuming model name does NOT collide with the source leaf
/// (`use_source.sql` reads `smelt.sources.raw.orders`), the typed path must
/// still produce concrete types. Guards against the fix accidentally breaking
/// the non-collision path.
#[test]
fn source_no_collision_still_typed() {
    let model_sql = "SELECT id, amount FROM smelt.sources.raw.orders\n";

    let tmp = stage_files(&[
        ("smelt.yml", SMELT_YML),
        ("models/use_source.sql", model_sql),
        ("models/sources/raw/orders.yml", SOURCE_YAML),
    ]);

    let (db, ws, use_source_file) = ingest(
        &tmp,
        &[("models/use_source.sql", model_sql)],
        "models/use_source.sql",
    );

    let schema = typed_model_schema(&db, ws, use_source_file);

    let col_map: std::collections::HashMap<_, _> = schema
        .columns
        .iter()
        .filter_map(|c| {
            c.data_type
                .as_ref()
                .map(|dt| (c.name.as_str(), dt.data_type.clone()))
        })
        .collect();

    assert_eq!(
        col_map.get("id"),
        Some(&DataType::Integer),
        "no-collision path: expected `id` to be Integer, got {:?}\nFull schema: {:#?}",
        col_map.get("id"),
        schema.columns
    );

    assert_eq!(
        col_map.get("amount"),
        Some(&DataType::Double),
        "no-collision path: expected `amount` to be Double, got {:?}\nFull schema: {:#?}",
        col_map.get("amount"),
        schema.columns
    );
}
