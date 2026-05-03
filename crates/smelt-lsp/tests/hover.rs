//! Phase 7 — hover on source column returns description; hover on seed column returns type.
//!
//! Test: `hover_on_source_column_returns_description`
//!   The `resolve_source` query used by the hover handler returns
//!   `SourceTableDef` whose column `description` fields are populated
//!   from the per-entity (or sources.yml) source YAML.
//!
//! Test: `hover_on_seed_column_returns_type`
//!   The `project_seeds` query used by the hover handler returns
//!   `SeedInfo` whose column types are inferred from the CSV. When a
//!   sidecar YAML is present, column descriptions are also included.
//!
//! These tests work at the smelt-db level and verify the data that the
//! LSP hover handler renders.

use std::path::PathBuf;

use smelt_db::{project_seeds, resolve_source, Database};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

fn build_db_with_sources(project_root: PathBuf, sources_yaml: &str) -> Database {
    let mut db = Database::default();
    db.set_project_input(project_root, sources_yaml.to_string());
    db.set_workspace(Vec::new(), Vec::new());
    db
}

// ---------------------------------------------------------------------------
// Test: hover_on_source_column_returns_description
// ---------------------------------------------------------------------------

/// `resolve_source` must return a `SourceTableDef` whose `columns` include
/// the `description` text declared in the sources YAML, so the hover handler
/// can render it.
#[test]
fn hover_on_source_column_returns_description() {
    let root = PathBuf::from("/fake/hover_project");

    let sources_yaml = r#"
sources:
  api:
    tables:
      orders:
        description: "Orders placed through the API"
        columns:
          - name: order_id
            type: INTEGER
            description: "Unique order identifier"
          - name: user_id
            type: INTEGER
            description: "Reference to the user who placed the order"
          - name: amount
            type: DOUBLE
"#;

    let db = build_db_with_sources(root.clone(), sources_yaml);
    let project = db
        .project_input(&root)
        .expect("project input must be registered");

    let table_def = resolve_source(&db, project, "api".to_string(), "orders".to_string())
        .expect("source 'api.orders' must resolve");

    // The table description should be populated.
    assert_eq!(
        table_def.description.as_deref(),
        Some("Orders placed through the API"),
        "table description must be returned from the source YAML"
    );

    // Verify column descriptions are included.
    let order_id = table_def
        .columns
        .iter()
        .find(|c| c.name == "order_id")
        .expect("order_id column expected");
    assert_eq!(
        order_id.description.as_deref(),
        Some("Unique order identifier"),
        "order_id description must be present"
    );

    let user_id = table_def
        .columns
        .iter()
        .find(|c| c.name == "user_id")
        .expect("user_id column expected");
    assert_eq!(
        user_id.description.as_deref(),
        Some("Reference to the user who placed the order"),
        "user_id description must be present"
    );

    // A column with no description returns None.
    let amount = table_def
        .columns
        .iter()
        .find(|c| c.name == "amount")
        .expect("amount column expected");
    assert_eq!(
        amount.description, None,
        "amount has no description, should be None"
    );
}

// ---------------------------------------------------------------------------
// Test: hover_on_seed_column_returns_type
// ---------------------------------------------------------------------------

/// `project_seeds` must return `SeedInfo` with column types inferred from the
/// CSV. When a sidecar YAML is present, column descriptions are also populated.
/// This pins the data contract between the seed data layer and the LSP hover
/// handler (which iterates `seed.columns` and `seed.sidecar.columns`).
#[test]
fn hover_on_seed_column_returns_type() {
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let project_root = tmp.path().to_path_buf();

    // Create models/lookup/ directory to hold the seed CSV.
    let lookup_dir = project_root.join("models").join("lookup");
    std::fs::create_dir_all(&lookup_dir).unwrap();

    // Write a CSV with typed columns (int, text, boolean).
    let csv_path = lookup_dir.join("regions.csv");
    std::fs::write(
        &csv_path,
        "region_id,region_name,is_active\n1,North,true\n2,South,false\n",
    )
    .unwrap();

    // Write a sidecar YAML with descriptions for region_id and region_name.
    let sidecar_path = lookup_dir.join("regions.yml");
    std::fs::write(
        &sidecar_path,
        "columns:\n\
         \x20 - name: region_id\n\
         \x20   type: INTEGER\n\
         \x20   description: \"Unique identifier for the region\"\n\
         \x20 - name: region_name\n\
         \x20   type: VARCHAR\n\
         \x20   description: \"Human-readable region label\"\n\
         \x20 - name: is_active\n\
         \x20   type: BOOLEAN\n",
    )
    .unwrap();

    // Write smelt.yml so project_seeds discovers seeds under models/.
    std::fs::write(
        project_root.join("smelt.yml"),
        "name: hover_seed_fixture\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n",
    )
    .unwrap();

    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());

    let seeds = project_seeds(&db, project);
    let seed = seeds
        .iter()
        .find(|s| s.address_segments == ["lookup", "regions"])
        .expect("seed 'lookup.regions' must be discovered");

    // Column types are inferred / pinned from the sidecar.
    let cols: std::collections::HashMap<_, _> = seed.columns.iter().cloned().collect();
    assert_eq!(
        cols["region_id"],
        smelt_types::DataType::Integer,
        "region_id should be INTEGER"
    );
    assert_eq!(
        cols["region_name"],
        smelt_types::DataType::Varchar { max_length: None },
        "region_name should be VARCHAR"
    );
    assert_eq!(
        cols["is_active"],
        smelt_types::DataType::Boolean,
        "is_active should be BOOLEAN"
    );

    // Sidecar descriptions are accessible for the hover handler.
    let sidecar = seed.sidecar.as_ref().expect("sidecar must be present");
    let sidecar_cols = sidecar
        .columns
        .as_ref()
        .expect("sidecar columns must be present");

    let region_id_col = sidecar_cols
        .iter()
        .find(|c| c.name == "region_id")
        .expect("region_id sidecar column expected");
    assert_eq!(
        region_id_col.description.as_deref(),
        Some("Unique identifier for the region"),
        "region_id description must come from the sidecar"
    );

    let region_name_col = sidecar_cols
        .iter()
        .find(|c| c.name == "region_name")
        .expect("region_name sidecar column expected");
    assert_eq!(
        region_name_col.description.as_deref(),
        Some("Human-readable region label"),
        "region_name description must come from the sidecar"
    );

    // Simulate what the LSP hover handler builds for a seed ref.
    let qualified_name = seed.address_segments.join(".");
    let mut content = format!("**Seed: {}**\n\n", qualified_name);
    content.push_str("Columns:\n");
    for (col_name, dtype) in &seed.columns {
        content.push_str(&format!("- `{}` ({})", col_name, dtype));
        if let Some(ref sc) = sidecar.columns {
            if let Some(c) = sc.iter().find(|c| &c.name == col_name) {
                if let Some(ref desc) = c.description {
                    content.push_str(&format!(" - {}", desc));
                }
            }
        }
        content.push('\n');
    }

    assert!(
        content.contains("Unique identifier for the region"),
        "hover content should include region_id description; got: {content:?}"
    );
    assert!(
        content.contains("Human-readable region label"),
        "hover content should include region_name description; got: {content:?}"
    );
}

/// The existing LSP hover code renders column descriptions in the markdown
/// output. Verify that the content string produced by the hover format logic
/// contains the expected description text.
#[test]
fn hover_content_includes_column_description() {
    let root = PathBuf::from("/fake/hover_content_project");

    let sources_yaml = r#"
sources:
  raw:
    tables:
      users:
        columns:
          - name: id
            type: INTEGER
            description: "Primary key for the users table"
          - name: email
            type: TEXT
            description: "User's email address"
"#;

    let db = build_db_with_sources(root.clone(), sources_yaml);
    let project = db.project_input(&root).expect("project must be registered");

    let table_def = resolve_source(&db, project, "raw".to_string(), "users".to_string())
        .expect("source 'raw.users' must resolve");

    // Simulate what the LSP hover handler builds.
    let mut content = format!("**Source: {}**\n\n", "raw.users");
    for col in &table_def.columns {
        content.push_str(&format!("- `{}`", col.name));
        if let Some(ref desc) = col.description {
            content.push_str(&format!(" - {}", desc));
        }
        content.push('\n');
    }

    assert!(
        content.contains("Primary key for the users table"),
        "hover content should include id description; got: {content:?}"
    );
    assert!(
        content.contains("User's email address"),
        "hover content should include email description; got: {content:?}"
    );
}
