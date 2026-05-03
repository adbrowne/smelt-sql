//! Phase 2: kind-by-content resolver tests.
//!
//! These tests verify that the walk_paths function classifies entities by
//! file format and content (not directory name), handles the .csv/.yml
//! sidecar tiebreaker, and detects cross-paths address collisions.

use smelt_core::resolver::{walk_paths, EntityKind, WorkspaceLoadError};
use std::fs;
use tempfile::TempDir;

fn write(dir: &std::path::Path, rel_path: &str, content: &str) {
    let full = dir.join(rel_path);
    if let Some(parent) = full.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(full, content).unwrap();
}

/// A workspace with paths: ["models"] and models/data/users.csv
/// produces a seed entity at address ["data", "users"].
#[test]
fn csv_resolves_to_seed() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "models/data/users.csv", "id,name\n1,Alice\n");
    let entities = walk_paths(tmp.path(), &["models".to_string()]).expect("walk_paths failed");

    assert_eq!(entities.len(), 1);
    assert_eq!(entities[0].address_segments, vec!["data", "users"]);
    assert!(
        matches!(entities[0].kind, EntityKind::Seed { sidecar: None }),
        "expected Seed with no sidecar, got {:?}",
        entities[0].kind
    );
}

/// models/data/users.csv + models/data/users.yml → seed with sidecar; no
/// source entity at ["data", "users"].
#[test]
fn csv_with_sibling_yml_is_seed_with_sidecar() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "models/data/users.csv", "id,name\n1,Alice\n");
    write(
        tmp.path(),
        "models/data/users.yml",
        "columns:\n  - name: id\n    type: INTEGER\n",
    );
    let entities = walk_paths(tmp.path(), &["models".to_string()]).expect("walk_paths failed");

    // Only one entity (the seed); no standalone source.
    assert_eq!(
        entities.len(),
        1,
        "expected exactly one entity: {entities:#?}"
    );
    assert_eq!(entities[0].address_segments, vec!["data", "users"]);
    match &entities[0].kind {
        EntityKind::Seed { sidecar: Some(_) } => {}
        other => panic!("expected Seed with sidecar, got {other:?}"),
    }
}

/// models/external/api/orders.yml (no sibling .csv) → source at
/// ["external", "api", "orders"].
#[test]
fn standalone_yml_is_source() {
    let tmp = TempDir::new().unwrap();
    write(
        tmp.path(),
        "models/external/api/orders.yml",
        "columns:\n  - name: order_id\n    type: INTEGER\n",
    );
    let entities = walk_paths(tmp.path(), &["models".to_string()]).expect("walk_paths failed");

    assert_eq!(entities.len(), 1, "expected one entity: {entities:#?}");
    assert_eq!(
        entities[0].address_segments,
        vec!["external", "api", "orders"]
    );
    assert_eq!(entities[0].kind, EntityKind::Source);
}

/// models/foo.sql with bare SELECT → Model;
/// models/bar.sql with smelt.define → Function.
#[test]
fn sql_bare_select_is_model_smelt_define_is_function() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "models/foo.sql", "SELECT id FROM raw_table\n");
    write(
        tmp.path(),
        "models/bar.sql",
        "smelt.define bar(x: Expr<Integer>) -> Expr<Integer> AS (x + 1)\n",
    );
    let entities = walk_paths(tmp.path(), &["models".to_string()]).expect("walk_paths failed");

    assert_eq!(entities.len(), 2, "expected two entities: {entities:#?}");

    let foo = entities
        .iter()
        .find(|e| e.address_segments == vec!["foo".to_string()])
        .expect("foo not found");
    assert_eq!(foo.kind, EntityKind::Model, "foo should be a model");

    let bar = entities
        .iter()
        .find(|e| e.address_segments == vec!["bar".to_string()])
        .expect("bar not found");
    assert_eq!(bar.kind, EntityKind::Function, "bar should be a function");
}

/// paths: ["models", "fixtures"] with models/users.csv and fixtures/users.csv
/// → hard workspace-load error naming both files.
#[test]
fn cross_paths_collision_errors() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "models/users.csv", "id,name\n1,a\n");
    write(tmp.path(), "fixtures/users.csv", "id,name\n2,b\n");

    let result = walk_paths(tmp.path(), &["models".to_string(), "fixtures".to_string()]);
    match result {
        Err(WorkspaceLoadError::DuplicateAddress {
            address,
            path1,
            path2,
        }) => {
            assert_eq!(address, vec!["users".to_string()]);
            // Both paths should be mentioned
            let p1_str = path1.to_string_lossy();
            let p2_str = path2.to_string_lossy();
            assert!(
                (p1_str.contains("models") && p2_str.contains("fixtures"))
                    || (p1_str.contains("fixtures") && p2_str.contains("models")),
                "expected model/fixture paths, got {} and {}",
                p1_str,
                p2_str
            );
        }
        other => panic!("expected DuplicateAddress error, got {other:?}"),
    }
}

/// data/users.csv and data/users.sql (bare SELECT) in the same dir
/// → workspace-load error (name collision within dir).
#[test]
fn name_collision_within_dir_errors() {
    let tmp = TempDir::new().unwrap();
    write(tmp.path(), "models/data/users.csv", "id,name\n1,a\n");
    write(
        tmp.path(),
        "models/data/users.sql",
        "SELECT id FROM somewhere\n",
    );

    let result = walk_paths(tmp.path(), &["models".to_string()]);
    match result {
        Err(WorkspaceLoadError::DuplicateAddress { address, .. }) => {
            assert_eq!(address, vec!["data", "users"]);
        }
        other => panic!("expected DuplicateAddress error, got {other:?}"),
    }
}
