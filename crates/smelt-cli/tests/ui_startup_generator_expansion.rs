//! BUG-077 regression: `smelt ui` startup omits generator model expansion.
//!
//! `commands/ui.rs` builds its `DependencyGraph` from
//! `discovery.discover_models()` only, without calling
//! `discover_emitted_model_files`. Generator-produced models are therefore
//! absent from the UI's graph and silently omitted from every run dispatched
//! via `execute_project` through the UI.
//!
//! The fix is to call `discover_emitted_model_files` during UI startup and
//! include the resulting emitted models when constructing the graph, mirroring
//! what `discover_models_for_run` does for the CLI run command.

use smelt_cli::{discover_emitted_model_files, init_db, Config, ModelDiscovery};
use smelt_core::graph::DependencyGraph;
use tempfile::TempDir;

/// Stage a minimal workspace with a generator file that emits two models.
/// The generator uses inline list syntax — no external YAML required.
fn stage_generator_workspace() -> TempDir {
    let tmp = TempDir::new().expect("create tempdir");
    let project_dir = tmp.path();

    // Inline list generator: emits "cohorts.east" and "cohorts.west".
    let generator = "---\ngenerates: models\n---\n\
        [ModelDef { name: 'east', body: SELECT 1 AS region_id }, \
         ModelDef { name: 'west', body: SELECT 2 AS region_id }]";
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    std::fs::write(project_dir.join("models/cohorts.gen.sql"), generator).unwrap();

    // A regular hand-authored model.
    std::fs::write(project_dir.join("models/base.sql"), "SELECT 42 AS id").unwrap();

    let yml = "name: ui_gen_test\n\
               version: 1\n\
               paths:\n  - models\n\
               targets:\n  dev:\n    type: duckdb\n    schema: main\n\
               default_materialization: view\n";
    std::fs::write(project_dir.join("smelt.yml"), yml).unwrap();

    tmp
}

/// BUG-077 regression: a graph built for the UI (same as `commands/ui.rs`)
/// MUST include generator-emitted models.
///
/// Before the fix, `commands/ui.rs` called `discovery.discover_models()` +
/// `DependencyGraph::build(raw_models, ...)` without invoking
/// `discover_emitted_model_files`. This test asserts the CORRECT behaviour so
/// it fails on the buggy code and passes after the fix.
#[test]
fn ui_startup_graph_includes_emitted_models() {
    let tmp = stage_generator_workspace();
    let project_dir = tmp.path();
    let config = Config::load(project_dir).expect("load config");
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());

    // ── Step 1: raw-only (the BUGGY startup) ─────────────────────────────────
    // This mirrors the current `commands/ui.rs` before the fix. Emitted models
    // are absent; running a project via the UI would silently skip them.
    let raw_models = discovery.discover_models().expect("discover raw models");
    let raw_graph = DependencyGraph::build(raw_models.clone(), None).expect("build raw graph");
    let raw_names = raw_graph.all_model_names();

    // Confirm the bug: raw-only graph lacks emitted models.
    assert!(
        !raw_names.contains("cohorts.east"),
        "pre-condition: raw-only graph must NOT contain 'cohorts.east' (confirms bug exists)"
    );
    assert!(
        !raw_names.contains("cohorts.west"),
        "pre-condition: raw-only graph must NOT contain 'cohorts.west' (confirms bug exists)"
    );

    // ── Step 2: fixed startup (raw + emitted) ─────────────────────────────────
    // This is what the fixed `commands/ui.rs` should do. The Salsa DB is
    // initialised with all SQL + function files so the meta-language evaluator
    // can expand the generator body; `discover_emitted_model_files` then
    // harvests the emitted `ModelFile` values and adds them to the graph.
    let function_files = discovery
        .discover_function_files()
        .expect("discover function files");
    let mut db_files = raw_models.clone();
    db_files.extend(function_files.iter().cloned());
    let db = init_db(project_dir, &db_files);

    let (emitted, _origins) = discover_emitted_model_files(&db, project_dir, &config.paths);
    assert!(
        !emitted.is_empty(),
        "discover_emitted_model_files must produce emitted models from the generator"
    );

    // Filter out generator files (their bodies are meta-language, not SQL) and
    // add emitted models — same as `discover_models_for_run` does for CLI runs.
    let mut expanded_models: Vec<smelt_core::ModelFile> = raw_models
        .into_iter()
        .filter(|m| m.metadata.as_ref().is_none_or(|md| md.generates.is_none()))
        .collect();
    expanded_models.extend(emitted);

    let fixed_graph = DependencyGraph::build(expanded_models, None).expect("build fixed graph");
    let fixed_names = fixed_graph.all_model_names();

    // The fixed graph must contain both emitted models.
    assert!(
        fixed_names.contains("cohorts.east"),
        "fixed graph must contain emitted model 'cohorts.east'; got: {:?}",
        fixed_names
    );
    assert!(
        fixed_names.contains("cohorts.west"),
        "fixed graph must contain emitted model 'cohorts.west'; got: {:?}",
        fixed_names
    );
    // Hand-authored model must still be present.
    assert!(
        fixed_names.contains("base"),
        "fixed graph must contain hand-authored 'base'; got: {:?}",
        fixed_names
    );
    // Generator file itself must NOT appear as an executable model.
    assert!(
        !fixed_names.contains("cohorts.gen") && !fixed_names.iter().any(|n| n.ends_with(".gen")),
        "generator file itself must NOT appear as an executable model; got: {:?}",
        fixed_names
    );
}
