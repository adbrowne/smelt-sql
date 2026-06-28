//! Regression guard for the workspace-keyed function-signature aggregation.
//!
//! `type_context` is computed once per file, and previously each computation
//! re-walked **every** workspace file calling `file_signature_inputs` to gather
//! `smelt.define` signatures. On an N-file workspace that made a cold
//! diagnostics pass O(N^2) in both wall-clock and Salsa dependency edges — the
//! `Salsa / Initial Load` and `Salsa / Full Diagnostics` benchmarks regressed
//! ~35–40% from this term alone at N=2000.
//!
//! The fix routes every per-file consumer through the single workspace-keyed
//! `workspace_function_signatures` query. These tests pin its behaviour so the
//! per-file walk cannot silently come back:
//!   1. it aggregates every `smelt.define` across the workspace,
//!   2. it is memoised per workspace (one Arc shared by all consumers), and
//!   3. a function **body** edit does not change its output (§20H hinge), so
//!      signature consumers are not invalidated by body-only edits.

use std::path::PathBuf;

use smelt_db::{workspace_function_signatures, Database, SourceFile, Workspace};

fn build_db(
    project_root: PathBuf,
    files: &[(PathBuf, String)],
) -> (Database, Workspace, Vec<SourceFile>) {
    let mut db = Database::default();
    let project = db.set_project_input(project_root.clone(), String::new());
    let mut handles = Vec::with_capacity(files.len());
    for (path, content) in files {
        let sf = db.set_source_file(path.clone(), content.clone(), project_root.clone());
        handles.push(sf);
    }
    db.set_workspace(handles.clone(), vec![project]);
    let ws = db.workspace();
    (db, ws, handles)
}

/// A `smelt.define` whose name is `f_{i}` with a scalar body.
fn define_src(i: usize) -> String {
    format!("smelt.define f_{i}(n: Expr<Integer>) AS (n)\n")
}

#[test]
fn workspace_function_signatures_aggregates_all_defines() {
    let root = PathBuf::from("/fake/project");
    let files = vec![
        (root.join("functions").join("a.sql"), define_src(0)),
        (root.join("functions").join("b.sql"), define_src(1)),
        // A plain model with no define contributes no signatures.
        (
            root.join("models").join("m.sql"),
            "SELECT 1 AS id\n".to_string(),
        ),
        (root.join("functions").join("c.sql"), define_src(2)),
    ];
    let (db, ws, _handles) = build_db(root, &files);

    let sigs = workspace_function_signatures(&db, ws);
    let mut names: Vec<&str> = sigs.iter().map(|s| s.name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        vec!["f_0", "f_1", "f_2"],
        "workspace_function_signatures must aggregate every smelt.define across the workspace"
    );
}

#[test]
fn workspace_function_signatures_is_memoized_per_workspace() {
    // Two consecutive calls in the same revision must return the *same* Arc:
    // the workspace-wide scan runs once and is shared by all per-file
    // consumers. If a per-file walk were reintroduced, callers would each
    // rebuild their own Vec instead of sharing this memoised one.
    let root = PathBuf::from("/fake/project");
    let files: Vec<(PathBuf, String)> = (0..8)
        .map(|i| {
            (
                root.join("functions").join(format!("f{i}.sql")),
                define_src(i),
            )
        })
        .collect();
    let (db, ws, _handles) = build_db(root, &files);

    let first = workspace_function_signatures(&db, ws);
    let second = workspace_function_signatures(&db, ws);
    assert!(
        std::sync::Arc::ptr_eq(&first, &second),
        "workspace_function_signatures must be memoised (shared Arc), not recomputed per call"
    );
    assert_eq!(first.len(), 8);
}

#[test]
fn body_edit_does_not_change_workspace_signatures() {
    // §20H: editing a function *body* must not change the aggregated signature
    // set, so signature consumers (type_context) are not invalidated. The
    // value must be content-equal after a body-only edit.
    let root = PathBuf::from("/fake/project");
    let fn_path = root.join("functions").join("f.sql");
    let other = root.join("functions").join("g.sql");
    let files = vec![
        (
            fn_path.clone(),
            "smelt.define keep(n: Expr<Integer>) AS (n)\n".to_string(),
        ),
        (other, define_src(9)),
    ];
    let (mut db, ws, _handles) = build_db(root.clone(), &files);

    let before: Vec<String> = workspace_function_signatures(&db, ws)
        .iter()
        .map(|s| s.name.clone())
        .collect();

    // Edit only the body of `keep` (signature unchanged).
    db.set_source_file(
        fn_path,
        "smelt.define keep(n: Expr<Integer>) AS (n + 1)\n".to_string(),
        root,
    );

    let after: Vec<String> = workspace_function_signatures(&db, ws)
        .iter()
        .map(|s| s.name.clone())
        .collect();

    assert_eq!(
        before, after,
        "a function body edit must not change the workspace signature set (§20H)"
    );
}
