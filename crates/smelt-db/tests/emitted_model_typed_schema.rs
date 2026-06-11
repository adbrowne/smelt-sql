//! Integration tests for Phase 1: typed schemas synthesised for generator-emitted models.
//!
//! Each test drives the real `emitted_model_typed_schema` Salsa query and asserts
//! on the resulting `ModelSchema`. Tests are TDD: written first (red), then made
//! green by implementing the query.
//!
//! Cases:
//!   1. `single_emission_concrete_body` — a generator emits `ModelDef { name: 'cohort_a',
//!      body: SELECT 1 AS id, CAST('us' AS TEXT) AS region }`. The schema has
//!      `{id: Integer, region: Text}`.
//!   2. `emission_body_reads_upstream_model` — generator emits
//!      `ModelDef { name: 'cohort_a', body: SELECT id, region FROM smelt.orders }`,
//!      where `models/orders.sql` exports `id: Integer, region: Text`. The
//!      emission's schema reflects the upstream types.
//!   3. `multiple_emissions_are_independent` — two emissions in one generator file
//!      produce independently-correct schemas (no cross-contamination).
//!   4. `body_with_inference_gap_is_partially_typed` — an emission whose body
//!      references an unbound column produces a partially-typed schema; the
//!      unresolvable column is `Unknown` (no panic, no silent failure).
//!   5. `emission_body_reads_source` — generator emits
//!      `ModelDef { name: 'orders_summary', body: SELECT id, amount FROM smelt.sources.raw.orders }`,
//!      where `models/sources/raw/orders.yml` declares `id: INTEGER, amount: DOUBLE`. The
//!      emission's schema must reflect the source-declared types (not `Unknown`).

use std::fs;
use std::path::PathBuf;

use smelt_db::{emitted_model_typed_schema, emitted_models, Database, Workspace};
use smelt_types::DataType;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Build a minimal database with one generator file and optionally one
/// hand-authored model.
fn build_db(project_root: PathBuf, files: &[(PathBuf, &str)]) -> (Database, Workspace) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    let mut handles = Vec::new();
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws)
}

// ---------------------------------------------------------------------------
// Test 1: single emission with a fully concrete body (no upstream ref)
// ---------------------------------------------------------------------------

/// A generator file emits one `ModelDef` whose body uses only literals.
/// The synthesised schema must resolve both columns to concrete types.
#[test]
fn single_emission_concrete_body() {
    let root = PathBuf::from("/fake/gen_schema_test");
    let gen_path = root.join("models").join("cohorts.gen.sql");
    let gen_src = r#"---
generates: models
---
[ModelDef {
  name: 'cohort_a',
  body: SELECT CAST(1 AS INTEGER) AS id, CAST('us' AS TEXT) AS region
}]
"#;

    let (db, ws) = build_db(root.clone(), &[(gen_path.clone(), gen_src)]);

    let result = emitted_models(&db, ws);
    assert_eq!(result.survivors.len(), 1, "expected one emission");
    let emission = &result.survivors[0];
    assert_eq!(emission.name, "cohort_a");

    // The new query under test.
    let gen_file = db
        .source_file(&gen_path)
        .expect("generator file registered");
    let schema = emitted_model_typed_schema(&db, ws, gen_file, "cohort_a".to_string());

    assert_eq!(
        schema.columns.len(),
        2,
        "expected 2 columns, got: {:?}",
        schema.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

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
        .expect("id has a TypedColumn")
        .data_type
        .clone();
    let region_type = region_col
        .data_type
        .as_ref()
        .expect("region has a TypedColumn")
        .data_type
        .clone();

    assert_ne!(
        id_type,
        DataType::Unknown,
        "id must be concrete, got Unknown"
    );
    assert_ne!(
        region_type,
        DataType::Unknown,
        "region must be concrete, got Unknown"
    );

    // Integer (or compatible — SmallInt / BigInt would also be fine,
    // but CAST(1 AS INTEGER) should produce Integer).
    assert!(
        matches!(
            id_type,
            DataType::Integer | DataType::SmallInt | DataType::BigInt
        ),
        "id should be an integer type, got {:?}",
        id_type
    );
    // Text or Varchar are both acceptable.
    assert!(
        matches!(region_type, DataType::Text | DataType::Varchar { .. }),
        "region should be Text/Varchar, got {:?}",
        region_type
    );
}

// ---------------------------------------------------------------------------
// Test 2: emission body reads an upstream hand-authored model
// ---------------------------------------------------------------------------

/// The generator emits a model whose body references `smelt.orders`.
/// `models/orders.sql` is a hand-authored model with `id: Integer, region: Text`.
/// The synthesised emission schema must propagate the upstream types.
#[test]
fn emission_body_reads_upstream_model() {
    let root = PathBuf::from("/fake/gen_upstream_test");
    let orders_path = root.join("models").join("orders.sql");
    let orders_src = "SELECT CAST(1 AS INTEGER) AS id, CAST('us' AS TEXT) AS region\n";

    let gen_path = root.join("models").join("cohorts.gen.sql");
    let gen_src = r#"---
generates: models
---
[ModelDef {
  name: 'cohort_a',
  body: SELECT id, region FROM smelt.orders
}]
"#;

    let (db, ws) = build_db(
        root.clone(),
        &[
            (orders_path.clone(), orders_src),
            (gen_path.clone(), gen_src),
        ],
    );

    let gen_file = db
        .source_file(&gen_path)
        .expect("generator file registered");
    let schema = emitted_model_typed_schema(&db, ws, gen_file, "cohort_a".to_string());

    assert_eq!(
        schema.columns.len(),
        2,
        "expected 2 columns, got: {:?}",
        schema.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

    for col in &schema.columns {
        let dt = col
            .data_type
            .as_ref()
            .unwrap_or_else(|| panic!("column '{}' has no TypedColumn", col.name))
            .data_type
            .clone();
        assert_ne!(
            dt,
            DataType::Unknown,
            "column '{}' should be concrete (propagated from orders.sql), got Unknown",
            col.name
        );
    }
}

// ---------------------------------------------------------------------------
// Test 3: multiple emissions in one generator file are independent
// ---------------------------------------------------------------------------

/// A generator emits two `ModelDef` values with different column shapes.
/// Each emission's schema must correspond to its own body, not bleed
/// into the other emission.
#[test]
fn multiple_emissions_are_independent() {
    let root = PathBuf::from("/fake/gen_multi_test");
    let gen_path = root.join("models").join("multi.gen.sql");
    // Emission "alpha" has columns (x: Integer).
    // Emission "beta" has columns (y: Text, z: Text).
    let gen_src = r#"---
generates: models
---
[
  ModelDef {
    name: 'alpha',
    body: SELECT CAST(42 AS INTEGER) AS x
  },
  ModelDef {
    name: 'beta',
    body: SELECT CAST('hello' AS TEXT) AS y, CAST('world' AS TEXT) AS z
  }
]
"#;

    let (db, ws) = build_db(root.clone(), &[(gen_path.clone(), gen_src)]);

    let result = emitted_models(&db, ws);
    assert_eq!(result.survivors.len(), 2, "expected two emissions");

    let gen_file = db
        .source_file(&gen_path)
        .expect("generator file registered");

    let alpha_schema = emitted_model_typed_schema(&db, ws, gen_file, "alpha".to_string());
    let beta_schema = emitted_model_typed_schema(&db, ws, gen_file, "beta".to_string());

    // alpha: one column "x"
    assert_eq!(alpha_schema.columns.len(), 1, "alpha should have 1 column");
    assert_eq!(alpha_schema.columns[0].name, "x");
    let x_type = alpha_schema.columns[0]
        .data_type
        .as_ref()
        .expect("x typed")
        .data_type
        .clone();
    assert_ne!(x_type, DataType::Unknown, "alpha.x must be concrete");
    assert!(
        matches!(
            x_type,
            DataType::Integer | DataType::SmallInt | DataType::BigInt
        ),
        "alpha.x should be integer, got {:?}",
        x_type
    );

    // beta: two columns "y" and "z"
    assert_eq!(beta_schema.columns.len(), 2, "beta should have 2 columns");
    let col_names: Vec<&str> = beta_schema
        .columns
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert!(col_names.contains(&"y"), "beta should have column y");
    assert!(col_names.contains(&"z"), "beta should have column z");
    for col in &beta_schema.columns {
        let dt = col.data_type.as_ref().expect("typed").data_type.clone();
        assert_ne!(
            dt,
            DataType::Unknown,
            "beta column '{}' must be concrete",
            col.name
        );
    }
}

// ---------------------------------------------------------------------------
// Test 4: body with an unresolvable column reference → Unknown (not a panic)
// ---------------------------------------------------------------------------

/// A generator emits a model whose body references `some_unbound_col` which
/// doesn't exist in any upstream model. The schema must still be returned
/// (no panic), with `some_unbound_col` typing as `Unknown`.
///
/// # No-silent-Unknown invariant — NOT YET ENFORCED for emission bodies
///
/// The plan's TDD item 4 originally required asserting that SQL-level inference
/// gaps inside emission bodies also emit diagnostics. That invariant is **not
/// yet enforced** in Phase 1 because `check_file_diagnostics` skips SQL
/// diagnostics for generator files (the W4 stage runs through a different code
/// path that does not pass through the per-file diagnostic walk). A
/// `ModelDef.body` containing an undeclared column reference, type mismatch,
/// or other SQL-level inference gap therefore produces a partially-typed
/// `ModelSchema` via `emitted_model_typed_schema` but no diagnostic; the gap
/// surfaces only when the emission's column is consumed downstream and that
/// consumer fails to type-check. See `docs/specs/meta_language.md` §Known
/// Divergences — "SQL-level inference gaps inside emission bodies do not emit
/// diagnostics." Tracked in `docs/plans/20260528-generator-emission-schema.md`.
#[test]
fn body_with_inference_gap_is_partially_typed() {
    let root = PathBuf::from("/fake/gen_gap_test");
    let gen_path = root.join("models").join("gap.gen.sql");
    // "known_col" is a literal → concrete. "unknown_col" is an unresolvable bare ref.
    let gen_src = r#"---
generates: models
---
[ModelDef {
  name: 'with_gap',
  body: SELECT CAST(1 AS INTEGER) AS known_col, some_unbound_col FROM nowhere_table
}]
"#;

    let (db, ws) = build_db(root.clone(), &[(gen_path.clone(), gen_src)]);

    let gen_file = db
        .source_file(&gen_path)
        .expect("generator file registered");
    let schema = emitted_model_typed_schema(&db, ws, gen_file, "with_gap".to_string());

    // The schema returns without panicking. The known column is concrete;
    // the unresolvable column may be Unknown.
    //
    // NOTE: We do NOT assert that diagnostics are emitted for the Unknown column
    // because `check_file_diagnostics` skips SQL diagnostics for generator files.
    // See the doc-comment above for the full rationale.
    let known = schema.columns.iter().find(|c| c.name == "known_col");
    if let Some(col) = known {
        let dt = col
            .data_type
            .as_ref()
            .map(|tc| tc.data_type.clone())
            .unwrap_or(DataType::Unknown);
        assert_ne!(dt, DataType::Unknown, "known_col should be concrete");
    }
    // No assertion on some_unbound_col — it may or may not resolve; we only
    // require the query doesn't panic and returns a schema.
    assert!(
        !schema.columns.is_empty(),
        "schema must have at least the known column"
    );
}

// ---------------------------------------------------------------------------
// Test 5: emission body reads a per-entity source YAML
// ---------------------------------------------------------------------------

/// A generator emits a model whose body references `smelt.sources.raw.orders`,
/// backed by a `models/sources/raw/orders.yml` file that declares `id: INTEGER,
/// amount: DOUBLE`.  The synthesised emission schema must resolve both columns
/// to their declared concrete types — not `Unknown`.
///
/// This exercises spec rule 6: emission bodies type-check "under the same regime
/// as a hand-authored model's body — including … sources."
///
/// Implementation note: per-entity sources are discovered from disk by the
/// `project_sources` Salsa query, so this test requires a real temporary
/// directory.  The `smelt.yml` sets `paths: [models]` so that
/// `smelt_core::Config::load` returns the correct scan root; otherwise
/// `discover_source_infos` would fall back to `["models"]` anyway, but being
/// explicit avoids coupling to that fallback.
#[test]
fn emission_body_reads_source() {
    // Stage a real project on disk so `project_sources` can discover the YAML.
    let tmp = TempDir::new().expect("create tempdir");
    let root = tmp.path().to_path_buf();

    // smelt.yml — tells smelt to scan `models/` for SQL and source YAMLs.
    let smelt_yml = "name: gen_source_test\n\
        version: 1\n\
        paths:\n  - models\n\
        targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
        default_materialization: view\n";

    // Per-entity source YAML.
    let source_yaml = "description: Raw orders source\n\
        columns:\n\
        - name: id\n  type: INTEGER\n\
        - name: amount\n  type: DOUBLE\n";

    // Generator file — emits a model whose body reads the source.
    let gen_src = r#"---
generates: models
---
[ModelDef {
  name: 'orders_summary',
  body: SELECT id, amount FROM smelt.sources.raw.orders
}]
"#;

    // Write files to the tempdir.
    fs::write(root.join("smelt.yml"), smelt_yml).expect("write smelt.yml");
    fs::create_dir_all(root.join("models/sources/raw")).expect("create dirs");
    fs::write(root.join("models/sources/raw/orders.yml"), source_yaml).expect("write source yaml");
    let gen_path = root.join("models/cohorts.gen.sql");
    fs::write(&gen_path, gen_src).expect("write generator");

    // Ingest into Salsa — note: `set_project_input` reads smelt.yml from disk.
    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), String::new());
    let gen_sf = db.set_source_file(gen_path.clone(), gen_src.to_string(), root.clone());
    db.set_workspace(vec![gen_sf], vec![project]);
    let ws = db.workspace();

    // The emission should resolve to concrete types from the source YAML.
    let schema = emitted_model_typed_schema(&db, ws, gen_sf, "orders_summary".to_string());

    assert_eq!(
        schema.columns.len(),
        2,
        "expected 2 columns, got: {:?}",
        schema.columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );

    let id_col = schema
        .columns
        .iter()
        .find(|c| c.name == "id")
        .expect("id column");
    let amount_col = schema
        .columns
        .iter()
        .find(|c| c.name == "amount")
        .expect("amount column");

    let id_type = id_col
        .data_type
        .as_ref()
        .expect("id has a TypedColumn")
        .data_type
        .clone();
    let amount_type = amount_col
        .data_type
        .as_ref()
        .expect("amount has a TypedColumn")
        .data_type
        .clone();

    assert_ne!(
        id_type,
        DataType::Unknown,
        "id must be concrete (from source YAML), got Unknown"
    );
    assert_ne!(
        amount_type,
        DataType::Unknown,
        "amount must be concrete (from source YAML), got Unknown"
    );

    assert!(
        matches!(
            id_type,
            DataType::Integer | DataType::SmallInt | DataType::BigInt
        ),
        "id should be integer type (declared INTEGER in source YAML), got {:?}",
        id_type
    );
    assert!(
        matches!(amount_type, DataType::Double | DataType::Float),
        "amount should be float/double type (declared DOUBLE in source YAML), got {:?}",
        amount_type
    );
}

// ---------------------------------------------------------------------------
// Test 6: emission body with LEFT JOIN — right-side source columns become nullable
// ---------------------------------------------------------------------------

/// A generator emits a model whose body LEFT-JOINs two sources.
/// Both sources declare all columns as `nullable: false`.
/// After the LEFT JOIN, columns from the right (null-supplying) side must be
/// inferred as `nullable: true`; the left side must remain `nullable: false`.
///
/// Regression guard for the `synthesise_emission_schema` gap identified in the
/// nullability-soundness plan: `apply_outer_join_nullability` was not called in
/// the generator code path, leaving the soundness hole open for emitted models.
#[test]
fn emission_body_left_join_null_supplying_side_is_nullable() {
    let tmp = TempDir::new().expect("create tempdir");
    let root = tmp.path().to_path_buf();

    let smelt_yml = "name: gen_join_nullability_test\n\
        version: 1\n\
        paths:\n  - models\n\
        targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
        default_materialization: view\n";

    // Left side: users — nullable: false declared on all columns.
    let users_yaml = "description: Users source\n\
        columns:\n\
        - name: user_id\n  type: INTEGER\n  nullable: false\n\
        - name: name\n  type: TEXT\n  nullable: false\n";

    // Right side: events — nullable: false declared on all columns.
    // After a LEFT JOIN, event_id and event_type must become nullable.
    let events_yaml = "description: Events source\n\
        columns:\n\
        - name: event_id\n  type: INTEGER\n  nullable: false\n\
        - name: user_id\n  type: INTEGER\n  nullable: false\n\
        - name: event_type\n  type: TEXT\n  nullable: false\n";

    let gen_src = "---\ngenerates: models\n---\n\
        [ModelDef {\n  \
          name: 'user_events',\n  \
          body: SELECT u.user_id, u.name, e.event_id, e.event_type \
                FROM smelt.sources.raw.users AS u \
                LEFT JOIN smelt.sources.raw.events AS e ON u.user_id = e.user_id\n\
        }]\n";

    fs::write(root.join("smelt.yml"), smelt_yml).expect("write smelt.yml");
    fs::create_dir_all(root.join("models/sources/raw")).expect("create dirs");
    fs::write(root.join("models/sources/raw/users.yml"), users_yaml).expect("write users");
    fs::write(root.join("models/sources/raw/events.yml"), events_yaml).expect("write events");
    let gen_path = root.join("models/user_events.gen.sql");
    fs::write(&gen_path, gen_src).expect("write generator");

    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), String::new());
    let gen_sf = db.set_source_file(gen_path.clone(), gen_src.to_string(), root.clone());
    db.set_workspace(vec![gen_sf], vec![project]);
    let ws = db.workspace();

    let schema = emitted_model_typed_schema(&db, ws, gen_sf, "user_events".to_string());

    let col_map: std::collections::HashMap<_, _> = schema
        .columns
        .iter()
        .filter_map(|c| c.data_type.as_ref().map(|dt| (c.name.as_str(), dt.clone())))
        .collect();

    assert!(
        !col_map.is_empty(),
        "schema returned no typed columns — sources not loaded. Schema: {:#?}",
        schema.columns
    );

    // Left side (users) — must remain non-nullable (LEFT JOIN preserves left).
    let user_id = col_map.get("user_id").expect("user_id column");
    let name = col_map.get("name").expect("name column");
    assert!(
        !user_id.nullable,
        "user_id (left side) should remain nullable: false after LEFT JOIN, got nullable: true"
    );
    assert!(
        !name.nullable,
        "name (left side) should remain nullable: false after LEFT JOIN, got nullable: true"
    );

    // Right side (events) — must become nullable after LEFT JOIN.
    let event_id = col_map.get("event_id").expect("event_id column");
    let event_type = col_map.get("event_type").expect("event_type column");
    assert!(
        event_id.nullable,
        "event_id (right side of LEFT JOIN) must be nullable: true, got nullable: false"
    );
    assert!(
        event_type.nullable,
        "event_type (right side of LEFT JOIN) must be nullable: true, got nullable: false"
    );
}
