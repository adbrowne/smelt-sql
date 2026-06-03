//! Diagnostic-parity (P6): a meta `List<T>` reaching a Data-World scalar /
//! SELECT-item position without being consumed is `MetaListInScalarPosition`.
//!
//! The design decision (forbid a bare list in scalar position; no implicit
//! auto-spread) is recorded in `docs/specs/meta_language.md` §Semantics "Lists
//! and spread" rule 10 and the List/spread diagnostic-codes table. The check is
//! a *select-shape* check that must run even for a model with no FROM clause
//! (a bare-meta SELECT was clean by accident before).

use std::path::PathBuf;

use smelt_db::{file_diagnostics, Database, DiagnosticCode, SourceFile, Workspace};

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

fn scalar_position_count(src: &str) -> usize {
    let root = PathBuf::from("/fake/project");
    let path = root.join("models").join("m.sql");
    let (db, ws, files) = build_db(root, &[(path, src)]);
    file_diagnostics(&db, ws, files[0])
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MetaListInScalarPosition))
        .count()
}

#[test]
fn bare_list_in_select_emits_scalar_position() {
    // A bare list literal as a SELECT item — FROM-less — is unconsumed.
    assert_eq!(
        scalar_position_count("SELECT [1, 2, 3]\n"),
        1,
        "a bare List<T> select item must emit exactly one MetaListInScalarPosition"
    );
}

#[test]
fn map_result_in_select_emits_scalar_position() {
    // `map`/`filter` produce a List<U>; left bare it is still unconsumed.
    assert_eq!(
        scalar_position_count("SELECT [1, 2, 3] |> map(fn c => c * 2)\n"),
        1,
        "a bare HOF (map) result in select position must emit MetaListInScalarPosition"
    );
}

#[test]
fn reduce_consumes_list_no_scalar_position() {
    // `reduce` collapses the list to a scalar — consumed, so no diagnostic.
    assert_eq!(
        scalar_position_count("SELECT reduce([1, 2, 3], plus_chain)\n"),
        0,
        "a list consumed by reduce must not emit MetaListInScalarPosition"
    );
}

#[test]
fn spread_does_not_emit_scalar_position() {
    // A spread is a consumer; the spread node is not a bare list select item.
    assert_eq!(
        scalar_position_count("SELECT id, ...[1, 2, 3]\nFROM smelt.sources.raw.users\n"),
        0,
        "a spread consumes the list; MetaListInScalarPosition must not fire"
    );
}

#[test]
fn bare_list_loader_in_select_emits_scalar_position() {
    // A `smelt.config.load_yaml(path, List<…>)` whose value is left bare in a
    // select item is an unconsumed `List<record>` — the same
    // `MetaListInScalarPosition` as a bare list literal (P7c design decision:
    // loaders must be consumed).
    assert_eq!(
        scalar_position_count(
            "SELECT smelt.config.load_yaml('configs/c.yaml', List<{name: Text}>)\n"
        ),
        1,
        "a bare List-loader select item must emit MetaListInScalarPosition"
    );
}

#[test]
fn bare_map_loader_in_select_emits_scalar_position() {
    // A `Map<Text, …>`-schema loader value is likewise a collection that must be
    // consumed before reaching a Data-World scalar position.
    assert_eq!(
        scalar_position_count(
            "SELECT smelt.config.load_yaml('configs/c.yaml', Map<Text, {plan: Text}>)\n"
        ),
        1,
        "a bare Map-loader select item must emit MetaListInScalarPosition"
    );
}

#[test]
fn spread_consumed_list_loader_no_scalar_position() {
    // A loader consumed by a spread + HOF is not a bare scalar-position list.
    assert_eq!(
        scalar_position_count(
            "SELECT ...smelt.config.load_yaml('configs/c.yaml', List<{name: Text}>) |> map(fn c => c.name)\n"
        ),
        0,
        "a spread-consumed loader list must not emit MetaListInScalarPosition"
    );
}

#[test]
fn heterogeneous_list_suppresses_scalar_position() {
    // A malformed (heterogeneous) bare list emits its own list diagnostic;
    // drop-on-error suppresses the scalar-position diagnostic (no double-emit).
    let root = PathBuf::from("/fake/project");
    let path = root.join("models").join("m.sql");
    let (db, ws, files) = build_db(root, &[(path, "SELECT [1, 'hello']\n")]);
    let diags = file_diagnostics(&db, ws, files[0]);
    let het = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MetaListHeterogeneous))
        .count();
    let scalar = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::MetaListInScalarPosition))
        .count();
    assert_eq!(
        het, 1,
        "heterogeneous bare list must emit MetaListHeterogeneous"
    );
    assert_eq!(
        scalar, 0,
        "MetaListInScalarPosition must be suppressed when a list error already fired"
    );
}
