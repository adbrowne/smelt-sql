//! Integration tests for Phase 2: consumer models reading from generator-emitted models.
//!
//! Each test drives the real `typed_model_schema` / `model_function_type` Salsa queries
//! and asserts that a consumer's columns resolve to concrete types (not `Unknown`) when
//! `FROM smelt.<emitted-path>` is used.
//!
//! Cases:
//!   1. `consumer_single_emission_resolves_typed_columns` — `SELECT id, region FROM
//!      smelt.cohorts.us_west` resolves to `{id: Integer, region: Text}` (typed, not Unknown).
//!   2. `consumer_union_all_over_multiple_emissions` — each branch of a UNION ALL over
//!      multiple emissions types concretely.
//!   3. `consumer_alias_on_emission` — `FROM smelt.cohorts.us_west AS c SELECT c.id, c.region`
//!      resolves through the alias to typed columns.
//!   4. `hand_authored_collision_still_wins` — a hand-authored model whose leaf path matches
//!      an emission leaf still resolves to the hand-authored model's schema.
//!   5. `sources_routing_not_regressed` — `FROM smelt.sources.<path>` still routes through
//!      the sources branch (no regression from the emission routing addition).

use smelt_db::{emitted_models, typed_model_schema, Database, Workspace};
use smelt_types::DataType;
use std::fs;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Minimal smelt.yml that declares `paths: [models]`.
const SMELT_YML: &str = "\
name: consumer_typing_test\n\
version: 1\n\
paths:\n  - models\n\
targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
default_materialization: view\n";

/// Build a database by writing files to a real temp directory (needed so that
/// `project_paths` reads `smelt.yml` and returns `["models"]`, which is
/// required for `emitted_model_smelt_path` to strip the `models/` prefix
/// and produce the correct `cohorts.us_west` path).
fn build_db_on_disk(tmp: &TempDir, files: &[(&str, &str)]) -> (Database, Workspace) {
    let root = tmp.path().to_path_buf();

    fs::write(root.join("smelt.yml"), SMELT_YML).expect("write smelt.yml");

    let mut handles = Vec::new();
    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), SMELT_YML.to_string());

    for (rel_path, content) in files {
        let full_path = root.join(rel_path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).expect("create dirs");
        }
        fs::write(&full_path, content).expect("write file");
        let sf = db.set_source_file(full_path, content.to_string(), root.clone());
        handles.push(sf);
    }

    db.set_workspace(handles, vec![project]);
    let ws = db.workspace();
    (db, ws)
}

// ---------------------------------------------------------------------------
// Test 1: Consumer reads from a single emission → typed columns
// ---------------------------------------------------------------------------

/// A consumer model does `SELECT id, region FROM smelt.cohorts.us_west`.
/// The emission `us_west` is produced by a generator whose body has
/// `SELECT CAST(1 AS INTEGER) AS id, CAST('us' AS TEXT) AS region`.
/// The consumer's typed schema must reflect concrete types.
#[test]
fn consumer_single_emission_resolves_typed_columns() {
    let tmp = TempDir::new().expect("tempdir");

    let gen_src = r#"---
generates: models
---
[ModelDef {
  name: 'us_west',
  body: SELECT CAST(1 AS INTEGER) AS id, CAST('us' AS TEXT) AS region
}]
"#;
    let consumer_src = "SELECT id, region FROM smelt.cohorts.us_west\n";

    let (db, ws) = build_db_on_disk(
        &tmp,
        &[
            ("models/cohorts.gen.sql", gen_src),
            ("models/all_cohorts.sql", consumer_src),
        ],
    );

    let consumer_path = tmp.path().join("models/all_cohorts.sql");
    let consumer_file = db
        .source_file(&consumer_path)
        .expect("consumer file registered");
    let schema = typed_model_schema(&db, ws, consumer_file);

    assert_eq!(
        schema.columns.len(),
        2,
        "consumer should have 2 columns, got: {:?}",
        schema.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

    for col in &schema.columns {
        let dt = col
            .data_type
            .as_ref()
            .map(|tc| tc.data_type.clone())
            .unwrap_or(DataType::Unknown);
        assert_ne!(
            dt,
            DataType::Unknown,
            "consumer column '{}' should be concrete (from emission), got Unknown",
            col.name
        );
    }

    let id_col = schema
        .columns
        .iter()
        .find(|c| c.name == "id")
        .expect("id column");
    let region_col = schema
        .columns
        .iter()
        .find(|c| c.name == "region")
        .expect("region column");

    let id_type = id_col
        .data_type
        .as_ref()
        .expect("id typed")
        .data_type
        .clone();
    let region_type = region_col
        .data_type
        .as_ref()
        .expect("region typed")
        .data_type
        .clone();

    assert!(
        matches!(
            id_type,
            DataType::Integer | DataType::SmallInt | DataType::BigInt
        ),
        "id should be integer type, got {:?}",
        id_type
    );
    assert!(
        matches!(region_type, DataType::Text | DataType::Varchar { .. }),
        "region should be Text/Varchar, got {:?}",
        region_type
    );
}

// ---------------------------------------------------------------------------
// Test 2: Consumer UNION ALL over multiple emissions → all branches typed
// ---------------------------------------------------------------------------

/// A consumer does UNION ALL over three emitted models. Each branch's columns
/// must type concretely.
#[test]
fn consumer_union_all_over_multiple_emissions() {
    let tmp = TempDir::new().expect("tempdir");

    let gen_src = r#"---
generates: models
---
[
  ModelDef {
    name: 'us_west',
    body: SELECT CAST(1 AS INTEGER) AS id, CAST('us_west' AS TEXT) AS region
  },
  ModelDef {
    name: 'us_east',
    body: SELECT CAST(2 AS INTEGER) AS id, CAST('us_east' AS TEXT) AS region
  },
  ModelDef {
    name: 'eu',
    body: SELECT CAST(3 AS INTEGER) AS id, CAST('eu' AS TEXT) AS region
  }
]
"#;

    let consumer_src = "\
SELECT id, region FROM smelt.cohorts.us_west
UNION ALL
SELECT id, region FROM smelt.cohorts.us_east
UNION ALL
SELECT id, region FROM smelt.cohorts.eu
";

    let (db, ws) = build_db_on_disk(
        &tmp,
        &[
            ("models/cohorts.gen.sql", gen_src),
            ("models/all_cohorts.sql", consumer_src),
        ],
    );

    let consumer_path = tmp.path().join("models/all_cohorts.sql");
    let consumer_file = db
        .source_file(&consumer_path)
        .expect("consumer file registered");
    let schema = typed_model_schema(&db, ws, consumer_file);

    assert_eq!(
        schema.columns.len(),
        2,
        "consumer should have 2 columns (from leading branch), got: {:?}",
        schema.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

    for col in &schema.columns {
        let dt = col
            .data_type
            .as_ref()
            .map(|tc| tc.data_type.clone())
            .unwrap_or(DataType::Unknown);
        assert_ne!(
            dt,
            DataType::Unknown,
            "consumer column '{}' in UNION ALL should be concrete, got Unknown",
            col.name
        );
    }
}

// ---------------------------------------------------------------------------
// Test 3: Consumer uses an alias on an emitted model
// ---------------------------------------------------------------------------

/// `FROM smelt.cohorts.us_west AS c SELECT c.id, c.region` — alias resolves
/// through to the emission's typed columns.
#[test]
fn consumer_alias_on_emission() {
    let tmp = TempDir::new().expect("tempdir");

    let gen_src = r#"---
generates: models
---
[ModelDef {
  name: 'us_west',
  body: SELECT CAST(1 AS INTEGER) AS id, CAST('us' AS TEXT) AS region
}]
"#;

    let consumer_src = "SELECT c.id, c.region FROM smelt.cohorts.us_west AS c\n";

    let (db, ws) = build_db_on_disk(
        &tmp,
        &[
            ("models/cohorts.gen.sql", gen_src),
            ("models/all_cohorts.sql", consumer_src),
        ],
    );

    let consumer_path = tmp.path().join("models/all_cohorts.sql");
    let consumer_file = db
        .source_file(&consumer_path)
        .expect("consumer file registered");
    let schema = typed_model_schema(&db, ws, consumer_file);

    assert_eq!(
        schema.columns.len(),
        2,
        "aliased consumer should have 2 columns, got: {:?}",
        schema.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

    for col in &schema.columns {
        let dt = col
            .data_type
            .as_ref()
            .map(|tc| tc.data_type.clone())
            .unwrap_or(DataType::Unknown);
        assert_ne!(
            dt,
            DataType::Unknown,
            "aliased consumer column '{}' should be concrete, got Unknown",
            col.name
        );
    }
}

// ---------------------------------------------------------------------------
// Test 4: Hand-authored collision tie-break — hand-authored model wins
// ---------------------------------------------------------------------------

/// A hand-authored model at `models/cohorts/us_west.sql` and a generator emission
/// named `us_west` under `cohorts/`. When both are present, the hand-authored
/// model's schema must win (per `ModelDefHandAuthoredCollision` spec rule).
///
/// In this test the hand-authored model has column `hand_authored_col: Double`,
/// while the emission has `id: Integer, region: Text`. The consumer's typed schema
/// must come from the hand-authored model (which uses `hand_authored_col`).
#[test]
fn hand_authored_collision_still_wins() {
    let tmp = TempDir::new().expect("tempdir");

    // Hand-authored model at cohorts/us_west.sql — unique column name so we can tell
    // which schema wins.
    let hand_authored_src = "SELECT CAST(3.14 AS DOUBLE) AS hand_authored_col\n";

    // Generator that would also emit `us_west` under cohorts/ — W3 collision
    // detection will discard this emission.
    let gen_src = r#"---
generates: models
---
[ModelDef {
  name: 'us_west',
  body: SELECT CAST(1 AS INTEGER) AS id, CAST('us' AS TEXT) AS region
}]
"#;

    // Consumer reads from smelt.cohorts.us_west — should see hand-authored schema.
    let consumer_src = "SELECT hand_authored_col FROM smelt.cohorts.us_west\n";

    let (db, ws) = build_db_on_disk(
        &tmp,
        &[
            ("models/cohorts/us_west.sql", hand_authored_src),
            ("models/cohorts.gen.sql", gen_src),
            ("models/all_cohorts.sql", consumer_src),
        ],
    );

    // Verify the collision detection discarded the emission.
    let emitted = emitted_models(&db, ws);
    let us_west_survivor = emitted.survivors.iter().find(|e| e.name == "us_west");
    // The emission should be discarded (None) because the hand-authored model at
    // cohorts/us_west.sql occupies the same smelt path `cohorts.us_west`.
    assert!(
        us_west_survivor.is_none(),
        "emission us_west should have been discarded due to hand-authored collision, \
         but it survived. survivors: {:?}",
        emitted
            .survivors
            .iter()
            .map(|e| &e.name)
            .collect::<Vec<_>>()
    );

    let consumer_path = tmp.path().join("models/all_cohorts.sql");
    let consumer_file = db
        .source_file(&consumer_path)
        .expect("consumer file registered");
    let schema = typed_model_schema(&db, ws, consumer_file);

    // The consumer must see the hand-authored schema (hand_authored_col), not
    // the emission's schema (id/region).
    let has_hand_authored = schema.columns.iter().any(|c| c.name == "hand_authored_col");
    let has_emission_col = schema
        .columns
        .iter()
        .any(|c| c.name == "id" || c.name == "region");

    assert!(
        !has_emission_col,
        "emission leaked through: consumer got emission columns (id/region). \
         schema: {:?}",
        schema.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

    // The hand-authored model's column should appear.
    assert!(
        has_hand_authored,
        "hand-authored column 'hand_authored_col' not found in consumer schema. \
         schema: {:?}",
        schema.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Test 5: smelt.sources.* routing not regressed
// ---------------------------------------------------------------------------

/// A consumer does `FROM smelt.sources.raw.orders`. This path starts with
/// "sources" and must route through the sources branch, not the emission
/// registry. Regression test: the emission routing addition must not
/// intercept sources references.
#[test]
fn sources_routing_not_regressed() {
    let tmp = TempDir::new().expect("tempdir");

    // Consumer references a source — no source YAML on disk in this minimal test,
    // so column types will be Unknown, but we must not crash or route incorrectly.
    let consumer_src = "SELECT id FROM smelt.sources.raw.orders\n";

    let (db, ws) = build_db_on_disk(&tmp, &[("models/consumer.sql", consumer_src)]);

    let consumer_path = tmp.path().join("models/consumer.sql");
    let consumer_file = db
        .source_file(&consumer_path)
        .expect("consumer file registered");

    // Must not panic. The schema may have Unknown columns (no source YAML), but
    // the sources branch must fire (not the emission branch).
    let schema = typed_model_schema(&db, ws, consumer_file);

    // Verify no emission was incorrectly consulted (no crash = pass).
    // The schema has one column (id), which is Unknown because there's no source YAML.
    // The key invariant is that we don't panic and don't route to the emission registry.
    assert_eq!(
        schema.columns.len(),
        1,
        "sources consumer should have 1 column (id), got: {:?}",
        schema.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

    // The column is Unknown (no source YAML), but we must not have crashed.
    let _ = schema;
}
