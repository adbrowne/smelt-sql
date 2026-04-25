//! Phase 42 — `smelt.as_struct()` backend-capability gate tests (tests 3
//! and 4 from the plan).
//!
//! These cover the diagnostic-emission half of Phase 42. SQL emission is
//! exercised by `crates/smelt-planner/tests/as_struct_lowering_tests.rs`;
//! Phase 38's prior behaviour (test 5 — explicit `backends:` declaration
//! preserved) is exercised by the unchanged
//! `crates/smelt-db/tests/as_struct_tests.rs::backend_without_struct_literal_errors`.
//!
//! Test 3: a function whose `backends:` declaration includes a backend
//! without struct-literal capability emits `AsStructUnsupportedBackend`.
//! Test 4: a function with NO `backends:` declaration (default
//! `BackendSet::All`) against a workspace whose active backends include a
//! non-struct-literal target also emits the diagnostic.

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};

fn build_db(
    project_root: PathBuf,
    smelt_yml: &str,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    db.set_project_smelt_yml(&project_root, smelt_yml.to_string());

    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

// ─── Phase 42 plan test 3: explicit backends + unsupported target ─────────────

#[test]
fn as_struct_unsupported_backend_errors() {
    // A function with an explicit `backends: [no_struct_db]` declaration
    // using `smelt.as_struct()` must emit `AsStructUnsupportedBackend`.
    // This is structurally identical to the Phase 38 fixture — the
    // capability gate is unchanged for `BackendSet::Only(...)` cases.
    let root = PathBuf::from("/fake/phase42_test3");
    let fn_path = root.join("models").join("fn_explicit.sql");
    let fn_src = "\
---\n\
backends: [no_struct_db]\n\
---\n\
smelt.define fn_explicit(source: TableExpr) -> TableExpr AS (\n\
    SELECT smelt.as_struct(source) AS s FROM source\n\
)\n";

    let (db, ws, files) = build_db(root, "", &[(fn_path, fn_src)]);
    let fn_file = files[0];

    let diags = file_diagnostics(&db, ws, fn_file);
    let matching: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::AsStructUnsupportedBackend)
                && d.message.contains("no_struct_db")
        })
        .collect();
    assert!(
        !matching.is_empty(),
        "expected AsStructUnsupportedBackend mentioning `no_struct_db`; got {diags:#?}"
    );
}

// ─── Sanity: yaml parses to active-backend list ──────────────────────────────

#[test]
fn project_active_backends_parses_targets_block() {
    use smelt_db::project_active_backends;

    let root = PathBuf::from("/fake/phase42_yml_parse");
    let smelt_yml = "name: phase42_yml_parse
version: 1
model_paths:
  - models
seed_paths:
  - seeds
targets:
  prod:
    type: no_struct_db
    schema: main
default_materialization: view
";

    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), String::new());
    db.set_project_smelt_yml(&root, smelt_yml.to_string());

    let backends = project_active_backends(&db, project);
    assert_eq!(
        backends,
        Some(vec!["no_struct_db".to_string()]),
        "smelt.yml's target_type should surface as the active backend list"
    );
}

// ─── Phase 42 plan test 4: default backends + workspace targets ───────────────

#[test]
fn as_struct_default_backends_capability_check_fires() {
    // The function has NO `backends:` frontmatter, so its declared set is
    // `BackendSet::All`. The workspace's `smelt.yml` declares a target
    // whose `target_type: no_struct_db` is not in the struct-literal
    // capability set. Phase 42 broadens the gate to consult the active
    // backends from `smelt.yml` when the declared set is `All`, so the
    // diagnostic must fire even though the function itself is silent
    // about backends.
    let root = PathBuf::from("/fake/phase42_test4");
    let fn_path = root.join("models").join("fn_default_backends.sql");
    let fn_src = "\
smelt.define fn_default_backends(source: TableExpr) -> TableExpr AS (\n\
    SELECT smelt.as_struct(source) AS s FROM source\n\
)\n";

    let smelt_yml = "name: phase42_test4
version: 1
model_paths:
  - models
seed_paths:
  - seeds
targets:
  prod:
    type: no_struct_db
    schema: main
default_materialization: view
";

    let (db, ws, files) = build_db(root, smelt_yml, &[(fn_path, fn_src)]);
    let fn_file = files[0];

    let diags = file_diagnostics(&db, ws, fn_file);
    let matching: Vec<_> = diags
        .iter()
        .filter(|d| {
            d.code == Some(DiagnosticCode::AsStructUnsupportedBackend)
                && d.message.contains("no_struct_db")
        })
        .collect();
    assert!(
        !matching.is_empty(),
        "expected AsStructUnsupportedBackend on default-backends function when \
         workspace targets a non-struct-literal backend; got {diags:#?}"
    );
}

// ─── Phase 38 regression: explicit Only(...) behaviour unchanged ─────────────
//
// Phase 42 plan test 5: with explicit `backends: [duckdb]`, the function
// must remain clean — the workspace's active set must not be applied
// when the function has restricted itself.

#[test]
fn as_struct_with_explicit_backends_only_unchanged() {
    let root = PathBuf::from("/fake/phase42_test5");
    let fn_path = root.join("models").join("fn_explicit_clean.sql");
    let fn_src = "\
---\n\
backends: [duckdb]\n\
---\n\
smelt.define fn_explicit_clean(source: TableExpr) -> TableExpr AS (\n\
    SELECT smelt.as_struct(source) AS s FROM source\n\
)\n";

    // Even though the workspace targets a non-struct-literal backend,
    // the function explicitly restricted itself to duckdb — Phase 42 must
    // not surface the workspace's broader active set as a diagnostic.
    let smelt_yml = "name: phase42_test5
version: 1
model_paths:
  - models
seed_paths:
  - seeds
targets:
  prod:
    type: no_struct_db
    schema: main
default_materialization: view
";

    let (db, ws, files) = build_db(root, smelt_yml, &[(fn_path, fn_src)]);
    let fn_file = files[0];

    let diags = file_diagnostics(&db, ws, fn_file);
    let as_struct_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::AsStructUnsupportedBackend))
        .collect();
    assert!(
        as_struct_diags.is_empty(),
        "explicit `backends: [duckdb]` must keep Phase 38 behaviour — \
         workspace's `no_struct_db` target must not fire the diagnostic; \
         got {as_struct_diags:#?}"
    );
}

#[test]
fn as_struct_default_backends_clean_when_workspace_supports_struct() {
    // Sanity: a default-backends function with smelt.as_struct() against
    // a workspace whose active backend IS struct-literal-capable
    // (duckdb) must remain clean. Guards against a regression where
    // we'd over-report on every default-backends function.
    let root = PathBuf::from("/fake/phase42_test4_clean");
    let fn_path = root.join("models").join("fn_default_backends_ok.sql");
    let fn_src = "\
smelt.define fn_default_backends_ok(source: TableExpr) -> TableExpr AS (\n\
    SELECT smelt.as_struct(source) AS s FROM source\n\
)\n";

    let smelt_yml = "name: phase42_test4_clean
version: 1
model_paths:
  - models
seed_paths:
  - seeds
targets:
  prod:
    type: duckdb
    database: target/dev.duckdb
    schema: main
default_materialization: view
";

    let (db, ws, files) = build_db(root, smelt_yml, &[(fn_path, fn_src)]);
    let fn_file = files[0];

    let diags = file_diagnostics(&db, ws, fn_file);
    let as_struct_diags: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::AsStructUnsupportedBackend))
        .collect();
    assert!(
        as_struct_diags.is_empty(),
        "duckdb-only workspace should not flag default-backends function; \
         got {as_struct_diags:#?}"
    );
}
