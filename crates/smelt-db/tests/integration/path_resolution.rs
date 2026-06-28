//! Phase 2a — `smelt-db` resolves `smelt.<path>` through the unified data
//! plane.
//!
//! Tests:
//!   1. `resolve_ref_path_form_returns_model_schema` — path-tuple resolution
//!      reaches a model file and produces its schema.
//!   2. `resolve_ref_kind_mismatch_on_test_path` — using `smelt.tests.*` in a
//!      `TableExpr` (FROM) position emits a kind-mismatch diagnostic.
//!   3. `tableexpr_substitution_through_path_arg` — passing `smelt.models.X`
//!      as a `TableExpr`-typed argument substitutes the model's schema into
//!      the function body's TypeContext, just like the legacy
//!      `smelt.ref('X')` form does.
//!   4. `legacy_smelt_ref_still_resolves` — the Phase-2a adapter keeps
//!      `smelt.ref('users')` resolving correctly until Phase 4 deletes the
//!      legacy parser path.

use std::path::PathBuf;
use std::sync::Arc;

use smelt_db::{
    file_diagnostics, model_schema, project_sql_address_index, resolve_ref_path,
    sorted_workspace_files, Database, DiagnosticCode, RefKind, ResolvedRef, SourceFile, Workspace,
};

#[test]
fn project_sql_address_index_maps_tuples_to_files() {
    // The workspace-keyed SQL address index maps each model's path tuple to
    // its (kind, file), so `resolve_ref_path` can look a ref up in O(1)
    // instead of rescanning every workspace file. This is the equivalence
    // contract the resolver relies on.
    let root = PathBuf::from("/fake/project");
    let users = root.join("models").join("users.sql");
    let orders = root.join("models").join("staging").join("orders.sql");

    let (db, ws, _files) = build_db(
        root.clone(),
        &[
            (users.clone(), "SELECT 1 AS id\n"),
            (orders.clone(), "SELECT 1 AS oid\n"),
        ],
    );
    let project = db.project_input(&root).expect("project registered");
    let index = project_sql_address_index(&db, ws, project);

    let u = index
        .get(&vec!["models".to_string(), "users".to_string()])
        .expect("users indexed under its path tuple");
    assert_eq!(u.0, RefKind::Model);
    assert_eq!(u.1.path(&db), &users);

    let o = index
        .get(&vec![
            "models".to_string(),
            "staging".to_string(),
            "orders".to_string(),
        ])
        .expect("nested model indexed under its full path tuple");
    assert_eq!(o.0, RefKind::Model);
    assert_eq!(o.1.path(&db), &orders);

    assert!(index
        .get(&vec!["models".to_string(), "missing".to_string()])
        .is_none());
}

#[test]
fn sorted_workspace_files_is_sorted_and_memoized() {
    // `sorted_workspace_files` returns the workspace's files ordered by
    // absolute path (byte-lexicographic), regardless of registration order,
    // and memoizes the result within a revision so all per-file/per-workspace
    // diagnostic queries can share one sorted list instead of re-sorting.
    let root = PathBuf::from("/fake/project");
    let zeta = root.join("models").join("zeta.sql");
    let alpha = root.join("models").join("alpha.sql");
    let mid = root.join("models").join("staging").join("mid.sql");

    // Register OUT of path order on purpose.
    let (db, ws, _files) = build_db(
        root,
        &[
            (zeta.clone(), "SELECT 1 AS z\n"),
            (alpha.clone(), "SELECT 1 AS a\n"),
            (mid.clone(), "SELECT 1 AS m\n"),
        ],
    );

    let sorted = sorted_workspace_files(&db, ws);
    let paths: Vec<&PathBuf> = sorted.iter().map(|f| f.path(&db)).collect();
    let mut expected = vec![&zeta, &alpha, &mid];
    expected.sort();
    assert_eq!(paths, expected, "files must be ordered by absolute path");

    // Two calls within one revision return the memoized Arc (pointer-eq).
    let sorted2 = sorted_workspace_files(&db, ws);
    assert!(
        Arc::ptr_eq(&sorted, &sorted2),
        "Salsa should memoize the sorted list within a revision (same Arc)"
    );
}

fn build_db(
    project_root: PathBuf,
    files: &[(PathBuf, &str)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), (*content).to_string(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

#[test]
fn resolve_ref_path_form_returns_model_schema() {
    // `smelt.models.users` resolves to the workspace's
    // `models/users.sql`. The path-tuple resolver returns a `ResolvedRef`
    // whose kind is `Model` and whose source-file path matches the file we
    // registered.
    let root = PathBuf::from("/fake/project");
    let users_path = root.join("models").join("users.sql");
    let users_src = "SELECT 1 AS id, 'a' AS name\n";

    let (db, ws, files) = build_db(root, &[(users_path.clone(), users_src)]);

    let resolved = resolve_ref_path(&db, ws, vec!["models".to_string(), "users".to_string()])
        .expect("resolve_ref_path returns ResolvedRef for known model path");

    assert_eq!(resolved.kind, RefKind::Model);
    let resolved_file = match &resolved {
        ResolvedRef {
            kind: RefKind::Model,
            source_file: Some(sf),
            ..
        } => *sf,
        other => panic!("expected Model with source file, got {other:?}"),
    };
    assert_eq!(resolved_file.path(&db), &users_path);

    // Schema lookup must work end-to-end through the same data plane.
    let schema = model_schema(&db, files[0]);
    let names: Vec<&str> = schema.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["id", "name"]);
}

#[test]
fn resolve_ref_kind_mismatch_on_test_path() {
    // A `.sql` file under `tests/` containing a `smelt.test` declaration cannot
    // appear in a `TableExpr` (FROM) position. Resolving the path tuple
    // succeeds with `RefKind::Test`, but using it in FROM emits a
    // kind-mismatch diagnostic.
    let root = PathBuf::from("/fake/project");
    let test_path = root.join("tests").join("foo.sql");
    let test_src = "smelt.test foo AS (SELECT 1 AS x WHERE 1 = 0)\nEXPECT ({x: 1})\n";
    let model_path = root.join("models").join("uses_test.sql");
    let model_src = "SELECT * FROM smelt.tests.foo\n";

    let (db, ws, files) = build_db(
        root,
        &[(test_path, test_src), (model_path.clone(), model_src)],
    );

    // Path resolves with kind Test.
    let resolved = resolve_ref_path(&db, ws, vec!["tests".to_string(), "foo".to_string()])
        .expect("test path resolves");
    assert_eq!(resolved.kind, RefKind::Test);

    // The model-using file emits a kind-mismatch diagnostic at the
    // path-form ref.
    let model_file = files[1];
    let diags = file_diagnostics(&db, ws, model_file);
    let kind_mismatches: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::KindMismatch))
        .collect();
    assert!(
        !kind_mismatches.is_empty(),
        "expected KindMismatch diagnostic for FROM smelt.tests.foo; got {diags:#?}"
    );
}

#[test]
fn tableexpr_substitution_through_path_arg() {
    // A function whose `TableExpr`-typed parameter is bound at the call
    // site to a path-form ref must see the referenced model's columns in
    // the body's TypeContext, just like the legacy `smelt.ref('X')` form.
    let root = PathBuf::from("/fake/project");
    let events_path = root.join("models").join("events.sql");
    let events_src = "SELECT 1 AS user_col, CURRENT_TIMESTAMP AS ts_col\n";

    // A function that consumes a `TableExpr` parameter and references its
    // columns in the body. We use `session_rollup`-shaped parameters but a
    // simpler body to avoid extra moving parts.
    let fn_path = root.join("functions").join("path_sub.sql");
    let fn_src = "\
smelt.define path_sub(
    source: TableExpr,
    user_col: Expr<Text>,
    ts_col: Expr<Timestamp>
) -> TableExpr AS (
    SELECT user_col, ts_col FROM source
)
";

    // The call site passes `smelt.models.events` (path form) as the
    // TableExpr argument. If substitution wires the events schema into
    // body's FROM-scope, the body's `user_col` / `ts_col` references
    // resolve cleanly and we see no body-level UnknownIdentifier.
    let caller_path = root.join("models").join("uses_path_sub.sql");
    let caller_src = "\
SELECT * FROM smelt.functions.path_sub(
    smelt.models.events,
    user_col,
    ts_col
)
";

    let (db, ws, files) = build_db(
        root,
        &[
            (events_path, events_src),
            (fn_path, fn_src),
            (caller_path, caller_src),
        ],
    );
    let caller_file = files[2];

    let diags = file_diagnostics(&db, ws, caller_file);
    let unknown_ident: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UnknownIdentifier))
        .collect();
    assert!(
        unknown_ident.is_empty(),
        "TableExpr substitution through path arg should resolve user_col/ts_col; \
         got {diags:#?}"
    );
}

#[test]
fn legacy_smelt_ref_is_parse_error() {
    // Phase 4: `smelt.ref('users')` is now a parse error. The parser must
    // reject it and emit a ParseError diagnostic. No UndefinedModelRef
    // should appear because the ref node is never constructed.
    let root = PathBuf::from("/fake/project");
    let users_path = root.join("models").join("users.sql");
    let users_src = "SELECT 1 AS id\n";
    let caller_path = root.join("models").join("downstream.sql");
    let caller_src = "SELECT * FROM smelt.ref('users')\n";

    let (db, ws, files) = build_db(
        root,
        &[(users_path.clone(), users_src), (caller_path, caller_src)],
    );
    let caller_file = files[1];

    // Phase 4: smelt.ref() must emit a parse error diagnostic.
    let diags = file_diagnostics(&db, ws, caller_file);
    let parse_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ParseError))
        .collect();
    assert!(
        !parse_errors.is_empty(),
        "smelt.ref('users') must produce a ParseError diagnostic in Phase 4; got {diags:#?}"
    );
}

/// D-08/D-09 lock: a bare unresolved `smelt.<path>` that is NOT under the
/// `smelt.sources.*` namespace routes to `UndefinedModelRef` (the generic
/// fallback), not to `UndefinedSource`.
#[test]
fn bare_unresolved_path_is_undefined_model_ref() {
    let root = PathBuf::from("/fake/routing");
    let model_path = root.join("models").join("anchor.sql");
    let model_src = "SELECT 1 AS id\n";
    // smelt.a.b.c — not under sources.* → UndefinedModelRef
    let caller_path = root.join("models").join("caller.sql");
    let caller_src = "SELECT id FROM smelt.a.b.c\n";

    let (db, ws, files) = build_db(root, &[(model_path, model_src), (caller_path, caller_src)]);
    let caller_file = files[1];
    let diags = file_diagnostics(&db, ws, caller_file);

    let undef_model: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UndefinedModelRef))
        .collect();
    let undef_source: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UndefinedSource))
        .collect();
    assert_eq!(
        undef_model.len(),
        1,
        "bare unresolved smelt.a.b.c must emit exactly one UndefinedModelRef; got {diags:#?}"
    );
    assert!(
        undef_source.is_empty(),
        "bare unresolved smelt.a.b.c must NOT emit UndefinedSource; got {diags:#?}"
    );
}

/// D-08/D-09 lock: an unresolved `smelt.sources.<schema>.<table>` path routes
/// to `UndefinedSource`, not to `UndefinedModelRef`.
#[test]
fn sources_namespace_unresolved_is_undefined_source() {
    let root = PathBuf::from("/fake/sources_routing");
    let model_path = root.join("models").join("anchor.sql");
    let model_src = "SELECT 1 AS id\n";
    // smelt.sources.raw.missing_table — no sources.yml → UndefinedSource
    let caller_path = root.join("models").join("caller.sql");
    let caller_src = "SELECT id FROM smelt.sources.raw.missing_table\n";

    let (db, ws, files) = build_db(root, &[(model_path, model_src), (caller_path, caller_src)]);
    let caller_file = files[1];
    let diags = file_diagnostics(&db, ws, caller_file);

    let undef_source: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UndefinedSource))
        .collect();
    let undef_model: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UndefinedModelRef))
        .collect();
    assert_eq!(
        undef_source.len(),
        1,
        "smelt.sources.raw.missing_table must emit exactly one UndefinedSource; got {diags:#?}"
    );
    assert!(
        undef_model.is_empty(),
        "smelt.sources.raw.missing_table must NOT emit UndefinedModelRef; got {diags:#?}"
    );
}

/// D-08/D-09 lock: the legacy `smelt.source(...)` call-form is a parse error
/// (rejected by the parser), not routed to UndefinedSource or UndefinedModelRef.
#[test]
fn legacy_smelt_source_is_parse_error() {
    let root = PathBuf::from("/fake/legacy_source");
    let model_path = root.join("models").join("anchor.sql");
    let model_src = "SELECT 1 AS id\n";
    let caller_path = root.join("models").join("caller.sql");
    // Legacy call-form: must be a parse error pointing to smelt.sources.*
    let caller_src = "SELECT * FROM smelt.source('raw.events')\n";

    let (db, ws, files) = build_db(root, &[(model_path, model_src), (caller_path, caller_src)]);
    let caller_file = files[1];
    let diags = file_diagnostics(&db, ws, caller_file);

    let parse_errors: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::ParseError))
        .collect();
    let undef_source: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UndefinedSource))
        .collect();
    assert!(
        !parse_errors.is_empty(),
        "smelt.source('raw.events') must produce a ParseError; got {diags:#?}"
    );
    assert!(
        undef_source.is_empty(),
        "smelt.source('raw.events') must NOT produce UndefinedSource; got {diags:#?}"
    );
}

/// Regression test: `resolve_ref_path` must not do work proportional to
/// `workspace.files * workspace.files` per call. Earlier, `file_path_tuple`
/// invoked `smelt_core::Config::load` (which performs disk I/O and YAML
/// parsing) once per workspace file inside the resolver, making each call
/// O(N) on disk and the per-file diagnostics phase O(N^2). On the 1000-model
/// CI bench this turned a sub-second pass into a multi-hour pass.
///
/// This test creates a workspace of 100 files, calls `file_diagnostics`
/// on every file, and asserts the total wall-clock time stays under a
/// generous threshold. Pre-fix, this takes tens of seconds; post-fix it
/// should complete in milliseconds.
#[test]
fn resolve_ref_path_does_not_scale_quadratically() {
    use std::time::Instant;
    use tempfile::TempDir;

    const N: usize = 200;

    let tmp = TempDir::new().expect("tmpdir");
    let root = tmp.path().to_path_buf();
    std::fs::create_dir_all(root.join("models")).expect("mkdir models");

    // Build a smelt.yml that mirrors the bench (one entry per model) so each
    // Config::load() parse pays a realistic cost. With the bug, this YAML is
    // re-parsed once per workspace file inside every resolve_ref_path call.
    let mut smelt_yml = String::from(
        "name: bench\nversion: 1\npaths:\n  - models\ntargets:\n  default:\n    type: duckdb\n    database: ./out.db\n    schema: main\nmodels:\n",
    );
    for i in 0..N {
        smelt_yml.push_str(&format!("  m_{i}:\n    materialization: table\n"));
    }
    std::fs::write(root.join("smelt.yml"), &smelt_yml).expect("write smelt.yml");

    let mut files: Vec<(PathBuf, String)> = Vec::with_capacity(N);
    for i in 0..N {
        let path = root.join("models").join(format!("m_{i}.sql"));
        let content = if i == 0 {
            "SELECT 1 AS id\n".to_string()
        } else {
            // Multiple path refs per file so the resolver runs more often.
            format!(
                "SELECT a.id\nFROM smelt.models.m_{p} a\nJOIN smelt.models.m_{p} b ON a.id = b.id\n",
                p = i - 1
            )
        };
        std::fs::write(&path, &content).expect("write model");
        files.push((path, content));
    }

    let mut db = Database::default();
    let project = db.set_project_input(root.clone(), smelt_yml.clone());
    let mut handles = Vec::with_capacity(N);
    for (path, content) in &files {
        let sf = db.set_source_file(path.clone(), content.clone(), root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();

    let start = Instant::now();
    for sf in &handles {
        let _ = file_diagnostics(&db, ws, *sf);
    }
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;

    // Pre-fix: thousands of ms even at N=100 (per-call cost grows with N).
    // Post-fix: low milliseconds.
    assert!(
        elapsed_ms < 2000.0,
        "file_diagnostics over {N} files took {elapsed_ms:.0} ms; \
         resolve_ref_path is doing per-file disk I/O again"
    );
}
