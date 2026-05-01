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

use smelt_db::{
    file_diagnostics, model_schema, resolve_ref_path, Database, DiagnosticCode, RefKind,
    ResolvedRef, SourceFile, Workspace,
};

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
    // A `.sql` file under `tests/` declaring a test (today carried by the
    // `materialization: test` frontmatter; spec uses `smelt.test`) cannot
    // appear in a `TableExpr` (FROM) position. Resolving the path tuple
    // succeeds with `RefKind::Test`, but using it in FROM emits a
    // kind-mismatch diagnostic.
    let root = PathBuf::from("/fake/project");
    let test_path = root.join("tests").join("foo.sql");
    let test_src = "\
---
materialization: test
---
SELECT 1 AS x WHERE 1 = 0
";
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
SELECT * FROM smelt.fn.path_sub(
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
