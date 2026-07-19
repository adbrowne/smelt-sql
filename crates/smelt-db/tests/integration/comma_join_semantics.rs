//! ANSI-89 implicit comma-separated FROM items (`FROM a, b`), classified as
//! a cross join (ratified 2026-07-18, master `docs/plans/20260718-quality-grind.md`
//! D-QG-2; docs/plans/20260718-quality-grind-t3.md Phase 2).
//!
//! `join_star_schema.rs::comma_join_schema_both_sides` covers `SELECT *`
//! expansion; this file covers type-checking a comma-joined `WHERE` filter
//! that correlates the two operands — the classic `FROM a, b WHERE a.x =
//! b.x` implicit-join-condition idiom.

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticSeverity, SourceFile, Workspace};

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
fn comma_join_where_filter_types() {
    // The classic ANSI-89 implicit-join idiom: the join condition lives in
    // WHERE, not in an ON clause. Both operands' columns must be visible to
    // the WHERE-clause type checker, and `a.x = b.x` (both INTEGER) must
    // type-check cleanly with no UnknownIdentifier/UndeclaredColumn errors.
    // Unaliased `smelt.models.a` / `smelt.models.b` refs bind to their leaf
    // segment (`a` / `b`), so bare `a.x` / `b.z` column refs resolve without
    // an explicit `AS` alias — same as any other unaliased FROM ref.
    let root = PathBuf::from("/fake/project/comma_join_where_filter_types");
    let a_path = root.join("models").join("a.sql");
    let b_path = root.join("models").join("b.sql");
    let m_path = root.join("models").join("m.sql");

    let (db, ws, files) = build_db(
        root,
        &[
            (a_path, "SELECT 1 AS x, 'p' AS y"),
            (b_path, "SELECT 1 AS x, 2.5 AS z"),
            (
                m_path,
                "SELECT a.x AS x, b.z AS z \
                 FROM smelt.models.a, smelt.models.b \
                 WHERE a.x = b.x",
            ),
        ],
    );
    let model_file = files[2];

    let diags = file_diagnostics(&db, ws, model_file);
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .collect();
    assert!(
        bad.is_empty(),
        "expected no errors type-checking a comma-join WHERE filter correlating both \
         operands; got {bad:#?}"
    );
}

#[test]
fn comma_join_aliased_where_filter_types() {
    // Same idiom with aliases, matching the common `FROM a, b WHERE a.x =
    // b.x` shorthand DuckDB/PostgreSQL programmers actually write.
    let root = PathBuf::from("/fake/project/comma_join_aliased_where_filter_types");
    let a_path = root.join("models").join("a.sql");
    let b_path = root.join("models").join("b.sql");
    let m_path = root.join("models").join("m.sql");

    let (db, ws, files) = build_db(
        root,
        &[
            (a_path, "SELECT 1 AS x, 'p' AS y"),
            (b_path, "SELECT 1 AS x, 2.5 AS z"),
            (
                m_path,
                "SELECT a.x AS x, b.z AS z \
                 FROM smelt.models.a AS a, smelt.models.b AS b \
                 WHERE a.x = b.x",
            ),
        ],
    );
    let model_file = files[2];

    let diags = file_diagnostics(&db, ws, model_file);
    let bad: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == DiagnosticSeverity::Error)
        .collect();
    assert!(
        bad.is_empty(),
        "expected no errors type-checking an aliased comma-join WHERE filter; got {bad:#?}"
    );
}
