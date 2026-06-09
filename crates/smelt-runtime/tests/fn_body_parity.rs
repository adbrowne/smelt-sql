//! Parity gate: `build_fn_body_map` (Salsa path) and
//! `build_fn_body_map_from_model_files` (model-files path) must produce an
//! identical `FnBodyMap` for the same workspace content.
//!
//! This test protects against the two entry points diverging as the shared
//! core in `fn_bodies.rs` evolves. Regression: if the inner implementations
//! are ever split again, this test will catch any drift.

use smelt_core::ModelId;
use smelt_runtime::{build_fn_body_map, build_fn_body_map_from_model_files};
use std::path::PathBuf;
use tempfile::TempDir;

const SAFE_DIVIDE_BODY: &str = "\
---
backends: [duckdb]
---
smelt.define safe_divide(numerator: Expr<Numeric>, denominator: Expr<Numeric>) -> Expr<Double>
    AS (CASE WHEN denominator = 0 OR denominator IS NULL THEN NULL ELSE CAST(numerator AS DOUBLE) / CAST(denominator AS DOUBLE) END)
";

const CLAMP_BODY: &str = "\
smelt.define clamp(v: Expr<Numeric>, lo: Expr<Numeric>, hi: Expr<Numeric>) -> Expr<Numeric>
    AS (GREATEST(lo, LEAST(hi, v)))
";

fn make_model_file(rel_path: &str, content: &str) -> smelt_core::ModelFile {
    let path = PathBuf::from(rel_path);
    smelt_core::ModelFile {
        name: path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        model_id: ModelId::from_path(path.clone()),
        path,
        content: content.to_string(),
        refs: vec![],
        parse_errors: vec![],
        metadata: None,
        kind: smelt_core::ModelKind::Sql,
        address_segments: vec![],
    }
}

fn setup_salsa_db(
    project_dir: &std::path::Path,
    files: &[(&str, &str)],
) -> (smelt_db::Database, smelt_db::Workspace) {
    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let source_files: Vec<_> = files
        .iter()
        .map(|(rel, content)| {
            let abs = project_dir.join(rel);
            db.set_source_file(abs, (*content).to_string(), project_dir.to_path_buf())
        })
        .collect();
    db.set_workspace(source_files, vec![project]);
    let workspace = smelt_db::Workspace::try_get(&db).expect("workspace not initialized");
    (db, workspace)
}

/// Both paths produce identical FnBodyMap entries for a workspace with two
/// function files — one with frontmatter, one without.
#[test]
fn both_paths_agree() {
    let tmp = TempDir::new().expect("create tempdir");
    let project_dir = tmp.path();

    let files = [
        ("functions/safe_divide.sql", SAFE_DIVIDE_BODY),
        ("functions/clamp.sql", CLAMP_BODY),
    ];

    // Create the files on disk for the Salsa path.
    for (rel, content) in &files {
        let abs = project_dir.join(rel);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(&abs, content).unwrap();
    }

    // --- Salsa path ---
    let (db, workspace) = setup_salsa_db(project_dir, &files);
    let salsa_map = build_fn_body_map(&db, workspace);

    // --- Model-files path ---
    let model_files: Vec<smelt_core::ModelFile> = files
        .iter()
        .map(|(rel, content)| make_model_file(rel, content))
        .collect();
    let files_map = build_fn_body_map_from_model_files(&model_files);

    // Both maps must agree on the full set of keys.
    let mut salsa_keys: Vec<&str> = salsa_map.keys().map(|s| s.as_str()).collect();
    let mut files_keys: Vec<&str> = files_map.keys().map(|s| s.as_str()).collect();
    salsa_keys.sort();
    files_keys.sort();
    assert_eq!(
        salsa_keys, files_keys,
        "key sets differ: salsa={salsa_keys:?} files={files_keys:?}"
    );

    // Each entry must agree on params and body SQL.
    for key in &salsa_keys {
        let salsa_entry = salsa_map.get(*key).unwrap();
        let files_entry = files_map.get(*key).unwrap();
        assert_eq!(
            salsa_entry.0, files_entry.0,
            "param list mismatch for '{key}': salsa={:?} files={:?}",
            salsa_entry.0, files_entry.0
        );
        assert_eq!(
            salsa_entry.1, files_entry.1,
            "body SQL mismatch for '{key}':\n  salsa: {:?}\n  files: {:?}",
            salsa_entry.1, files_entry.1
        );
    }
}

/// An empty workspace produces an empty map from both paths.
#[test]
fn empty_workspace_returns_empty_map() {
    let tmp = TempDir::new().expect("create tempdir");
    let project_dir = tmp.path();

    let (db, workspace) = setup_salsa_db(project_dir, &[]);
    let salsa_map = build_fn_body_map(&db, workspace);
    let files_map = build_fn_body_map_from_model_files(&[]);

    assert!(salsa_map.is_empty(), "Salsa path: expected empty map");
    assert!(files_map.is_empty(), "Files path: expected empty map");
}
