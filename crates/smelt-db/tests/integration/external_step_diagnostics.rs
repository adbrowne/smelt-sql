//! `project_source_diagnostics` surfaces `MalformedExternalStep` and
//! `SourceProducerConflict` for external-step declarations.
//!
//! Spec: `docs/specs/sources.md` §"Externally-produced sources (black-box
//! steps)". Plan: `docs/outcomes/20260906-external-dag-steps/phases/02-plan.md`.
//!
//! `project_source_diagnostics` discovers step/source YAML from the real
//! filesystem (mirroring `discover_source_errors`), so these tests use a real
//! `TempDir` rather than the virtual Salsa source-file inputs other
//! diagnostics tests use.

use std::fs;
use std::path::PathBuf;

use smelt_db::{
    project_source_diagnostics, resolve_node_path, resolve_ref_path, Database, DiagnosticCode,
};
use tempfile::TempDir;

fn make_workspace(tmp: &TempDir) -> (Database, smelt_db::ProjectInput) {
    let root = tmp.path().to_path_buf();
    fs::write(root.join("smelt.yml"), "name: test\npaths:\n  - models\n").unwrap();
    fs::create_dir_all(root.join("models")).unwrap();
    fs::write(root.join("models/m.sql"), "SELECT 1 AS x").unwrap();

    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), String::new());
    let sf = db.set_source_file(
        root.join("models/m.sql"),
        "SELECT 1 AS x".to_string(),
        root.clone(),
    );
    db.set_workspace(vec![sf], vec![project]);
    (db, project)
}

#[test]
fn malformed_step_yields_malformed_external_step() {
    let tmp = TempDir::new().unwrap();
    let (db, project) = make_workspace(&tmp);
    let root = tmp.path();

    fs::create_dir_all(root.join("models/sources")).unwrap();
    fs::write(
        root.join("models/sources/step.yml"),
        "external_step:\n  command: [\"bash\", \"loader.sh\"]\n",
    )
    .unwrap();

    let diags = project_source_diagnostics(&db, project);
    let matches: Vec<_> = diags
        .iter()
        .filter(|d| d.diagnostic.code == Some(DiagnosticCode::MalformedExternalStep))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one MalformedExternalStep, got: {diags:?}"
    );
    assert_eq!(matches[0].path, root.join("models/sources/step.yml"));
}

#[test]
fn duplicate_producer_yields_source_producer_conflict() {
    let tmp = TempDir::new().unwrap();
    let (db, project) = make_workspace(&tmp);
    let root = tmp.path();

    fs::create_dir_all(root.join("models/sources")).unwrap();
    fs::write(
        root.join("models/sources/orders.yml"),
        "columns:\n  - { name: order_id, type: INTEGER, nullable: false }\n",
    )
    .unwrap();
    fs::write(
        root.join("models/sources/step_a.yml"),
        "external_step:\n  produces:\n    - smelt.sources.orders\n  command: [\"bash\", \"a.sh\"]\n",
    )
    .unwrap();
    fs::write(
        root.join("models/sources/step_b.yml"),
        "external_step:\n  produces:\n    - smelt.sources.orders\n  command: [\"bash\", \"b.sh\"]\n",
    )
    .unwrap();

    let diags = project_source_diagnostics(&db, project);
    let matches: Vec<_> = diags
        .iter()
        .filter(|d| d.diagnostic.code == Some(DiagnosticCode::SourceProducerConflict))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one SourceProducerConflict, got: {diags:?}"
    );

    let both_paths_named = matches[0].diagnostic.message.contains("step_a.yml")
        && matches[0].diagnostic.message.contains("step_b.yml");
    assert!(
        both_paths_named,
        "conflict message should name both files: {}",
        matches[0].diagnostic.message
    );

    let expected: PathBuf = root.join("models/sources/step_b.yml");
    assert_eq!(
        matches[0].path, expected,
        "conflict should be anchored at the later-sorted step"
    );
}

/// `resolve_node_path` (ref resolution ∪ external steps) resolves a step's
/// own address as a node, even though `resolve_ref_path` never does — the
/// two-seam split (`model_selection.md` §"Constraints & Invariants" — a step
/// is a selectable node but never a `smelt.ref()` target).
#[test]
fn resolve_node_path_resolves_a_step() {
    let tmp = TempDir::new().unwrap();
    let (mut db, _project) = make_workspace(&tmp);
    let root = tmp.path();

    fs::create_dir_all(root.join("models/sources")).unwrap();
    fs::write(
        root.join("models/sources/raw_events.yml"),
        "columns:\n  - name: id\n    type: INTEGER\n",
    )
    .unwrap();
    fs::write(
        root.join("models/loader.yml"),
        "external_step:\n  produces:\n    - smelt.sources.raw_events\n  command: [\"bash\", \"loader.sh\"]\n",
    )
    .unwrap();

    let workspace = db.workspace();
    let path = vec!["loader".to_string()];

    assert!(
        resolve_ref_path(&db, workspace, path.clone()).is_none(),
        "a step is not a smelt.ref() target — resolve_ref_path must stay step-free"
    );
    assert!(
        resolve_node_path(&db, workspace, path),
        "resolve_node_path must resolve a step's own address"
    );
}

/// A SQL `smelt.ref()` naming a step's address is not a step reference — it
/// fails `UndefinedModelRef` via `resolve_ref_path`, since a model reads the
/// source a step produces, never the step itself.
#[test]
fn sql_ref_to_a_step_is_undefined() {
    let tmp = TempDir::new().unwrap();
    let (mut db, _project) = make_workspace(&tmp);
    let root = tmp.path();

    fs::create_dir_all(root.join("models/sources")).unwrap();
    fs::write(
        root.join("models/sources/raw_events.yml"),
        "columns:\n  - name: id\n    type: INTEGER\n",
    )
    .unwrap();
    fs::write(
        root.join("models/loader.yml"),
        "external_step:\n  produces:\n    - smelt.sources.raw_events\n  command: [\"bash\", \"loader.sh\"]\n",
    )
    .unwrap();

    let workspace = db.workspace();
    assert!(resolve_ref_path(&db, workspace, vec!["loader".to_string()]).is_none());
}
