//! `Maintenance*` diagnostics: the thin `maintenance_plan` Salsa query
//! (`crates/smelt-db/src/queries/maintenance.rs`) folds the derived plan's
//! admission refusals, plus the `maintenance.cells[]` column-group-span
//! check, into `file_diagnostics()`.
//!
//! Spec: `docs/specs/incremental_models.md` §Diagnostics, §Semantics
//! "Partition-local maintenance (the K8 guardrail)"; `docs/specs/models.md`
//! "Declared grain contradicted by the derived plan" (Constraint violations
//! table).

use std::fs;

use smelt_core::workspace::load_workspace;
use smelt_db::{workspace_ingest::ingest_loaded_workspace, DiagnosticCode};

/// Build a real on-disk workspace under a fresh tempdir, ingest it into a
/// Salsa `Database`, and return the diagnostics for `model_file` (relative
/// to `models/`, without extension).
fn diagnostics_for(files: &[(&str, &str)], model_file: &str) -> Vec<smelt_db::Diagnostic> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    diagnostics_for_in(&root, files, model_file)
}

/// Like [`diagnostics_for`], but ingests against a caller-supplied root —
/// lets a test stage a `.smelt/` deployed-schema snapshot at the same root
/// before ingest sees it (`diagnostics_for` creates its own private tempdir,
/// which a caller can never write into ahead of time).
fn diagnostics_for_in(
    root: &std::path::Path,
    files: &[(&str, &str)],
    model_file: &str,
) -> Vec<smelt_db::Diagnostic> {
    for (rel, content) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
    }

    let loaded = load_workspace(root);
    let mut db = smelt_db::Database::default();
    let ingested = ingest_loaded_workspace(&mut db, &loaded);
    db.set_workspace(ingested.source_files.clone(), vec![ingested.project]);
    let ws = db.workspace();

    let target_path = root.join("models").join(format!("{model_file}.sql"));
    let file = ingested
        .source_files
        .iter()
        .zip(ingested.paths.iter())
        .find(|(_, p)| **p == target_path)
        .map(|(f, _)| *f)
        .unwrap_or_else(|| panic!("model file {target_path:?} not ingested"));

    smelt_db::file_diagnostics(&db, ws, file)
}

/// Like [`diagnostics_for`], but returns the derived
/// [`smelt_db::queries::maintenance::MaintenancePlanResult`] itself
/// (`smelt_db::maintenance_plan_report`) rather than diagnostics — for
/// asserting cell-level shape (technique, locality, scans) through the SAME
/// production Salsa wrapper `file_diagnostics` consumes.
fn plan_for(
    files: &[(&str, &str)],
    model_file: &str,
) -> smelt_db::queries::maintenance::MaintenancePlanResult {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    plan_for_in(&root, files, model_file)
}

/// Like [`plan_for`], but ingests against a caller-supplied root — lets a
/// test stage a `.smelt/` deployed-schema snapshot at the same root before
/// ingest sees it, mirroring [`diagnostics_for_in`]'s relationship to
/// [`diagnostics_for`].
fn plan_for_in(
    root: &std::path::Path,
    files: &[(&str, &str)],
    model_file: &str,
) -> smelt_db::queries::maintenance::MaintenancePlanResult {
    for (rel, content) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, content).unwrap();
    }

    let loaded = load_workspace(root);
    let mut db = smelt_db::Database::default();
    let ingested = ingest_loaded_workspace(&mut db, &loaded);
    db.set_workspace(ingested.source_files.clone(), vec![ingested.project]);
    let ws = db.workspace();

    let target_path = root.join("models").join(format!("{model_file}.sql"));
    let file = ingested
        .source_files
        .iter()
        .zip(ingested.paths.iter())
        .find(|(_, p)| **p == target_path)
        .map(|(f, _)| *f)
        .unwrap_or_else(|| panic!("model file {target_path:?} not ingested"));

    smelt_db::maintenance_plan_report(&db, ws, file)
        .unwrap_or_else(|| panic!("model {model_file} has no maintenance plan"))
}

const SMELT_YML: &str = r#"
name: maintenance_diagnostics_fixture
version: 1

paths:
  - models

targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main

default_materialization: view
"#;

fn walk_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_files(&path));
        } else if path.is_file() {
            out.push(path);
        }
    }
    out
}

mod column_added_and_keyed;
mod deployed_schema_world_fact_module;
mod scan_bounds_and_grain;
mod status_and_contract;
