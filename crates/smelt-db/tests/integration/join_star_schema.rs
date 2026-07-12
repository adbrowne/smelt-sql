//! `SELECT *` schema expansion across joins — NATURAL / USING dedupe.
//!
//! DuckDB-verified semantics (a(x, y), b(x, z)):
//! - `SELECT * FROM a NATURAL JOIN b`    → 3 columns [x, y, z]: the shared
//!   column appears once (left side's occurrence).
//! - `SELECT * FROM a JOIN b USING (x)`  → 3 columns [x, y, z]: same dedupe,
//!   keyed by the USING column list.
//! - `SELECT * FROM a JOIN b ON …`       → 4 columns [x, y, x, z]: ON joins
//!   do NOT dedupe.
//!
//! smelt's `model_schema` wildcard expansion (row_extensions) covers the
//! NATURAL and USING forms: the joined table's columns are expanded with the
//! join-shared columns deduped (first occurrence wins — the left side's
//! type). The ON form is NOT yet expanded across the join at all (the right
//! side's columns are absent from the inferred schema — a pre-existing gap,
//! tracked in docs/TODO.md); the guard test below pins the current
//! (incomplete, but not-duplicated) behavior so any change to it is a
//! conscious one.

use std::path::PathBuf;

use smelt_db::{resolved_model_schema, Database, SourceFile, Workspace};

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

/// Resolve the star-expanded schema of a model over upstream models
/// a(x, y) and b(x, z); returns the resolved column names in order.
fn star_schema_columns(model_sql: &str, test_name: &str) -> Vec<String> {
    let root = PathBuf::from(format!("/fake/project/{}", test_name));
    let a_path = root.join("models").join("a.sql");
    let b_path = root.join("models").join("b.sql");
    let m_path = root.join("models").join("m.sql");

    let (db, ws, files) = build_db(
        root,
        &[
            (a_path, "SELECT 1 AS x, 'p' AS y"),
            (b_path, "SELECT 1 AS x, 2.5 AS z"),
            (m_path, model_sql),
        ],
    );
    let schema = resolved_model_schema(&db, ws, files[2]);
    assert!(
        schema.is_fully_resolved,
        "star schema for {test_name} should fully resolve"
    );
    schema.columns.iter().map(|c| c.name.clone()).collect()
}

#[test]
fn natural_join_star_dedupes_shared_columns() {
    let cols = star_schema_columns(
        "SELECT * FROM smelt.models.a NATURAL JOIN smelt.models.b",
        "natural_join_star",
    );
    assert_eq!(
        cols,
        vec!["x", "y", "z"],
        "NATURAL JOIN star expansion must be [x, y, z] (shared column deduped, \
         DuckDB-verified), got {cols:?}"
    );
}

#[test]
fn using_join_star_dedupes_using_columns() {
    let cols = star_schema_columns(
        "SELECT * FROM smelt.models.a JOIN smelt.models.b USING (x)",
        "using_join_star",
    );
    assert_eq!(
        cols,
        vec!["x", "y", "z"],
        "JOIN … USING (x) star expansion must be [x, y, z] (USING column deduped, \
         DuckDB-verified), got {cols:?}"
    );
}

#[test]
fn natural_join_star_keeps_left_side_type_for_shared_column() {
    let root = PathBuf::from("/fake/project/natural_join_star_type");
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
                "SELECT * FROM smelt.models.a NATURAL JOIN smelt.models.b",
            ),
        ],
    );
    let schema = resolved_model_schema(&db, ws, files[2]);
    let x = schema
        .columns
        .iter()
        .find(|c| c.name == "x")
        .expect("shared column x present");
    let a_schema = resolved_model_schema(&db, ws, files[0]);
    let a_x = a_schema
        .columns
        .iter()
        .find(|c| c.name == "x")
        .expect("a has x");
    assert_eq!(
        x.data_type, a_x.data_type,
        "deduped shared column keeps the left (first-occurrence) side's type"
    );
}

#[test]
fn on_join_star_current_behavior_left_side_only() {
    // Pre-existing gap, pinned: ON joins do not yet expand the right side's
    // columns into `SELECT *` at all (DuckDB would give [x, y, x, z]).
    // Tracked in docs/TODO.md — if this test starts failing because the
    // right side is now included, update the expectation to the DuckDB
    // duplicated form, not a deduped one.
    let cols = star_schema_columns(
        "SELECT * FROM smelt.models.a JOIN smelt.models.b ON a.x = b.x",
        "on_join_star",
    );
    assert_eq!(
        cols,
        vec!["x", "y"],
        "ON-join star expansion currently covers the left side only, got {cols:?}"
    );
}

#[test]
fn single_table_star_unchanged() {
    // Guard: the join-extension work must not disturb the plain single-table
    // star expansion.
    let cols = star_schema_columns("SELECT * FROM smelt.models.a", "single_table_star");
    assert_eq!(cols, vec!["x", "y"]);
}
