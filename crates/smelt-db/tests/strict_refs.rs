//! Phase 2 — Strict full-path resolution for `smelt.<path>` refs in model SQL.
//!
//! Tests:
//!   1. `leaf_only_ref_is_undefined` — `FROM smelt.events_parsed` (leaf-only,
//!      missing `silver.`) emits `UndefinedModelRef` with a "did you mean
//!      'smelt.silver.events_parsed'?" hint.
//!   2. `correct_full_path_resolves` — `FROM smelt.silver.events_parsed` (fully
//!      qualified) produces no diagnostics.
//!   3. `ambiguous_leaf_no_hint` — two models share the same leaf name; the
//!      error lists both candidates.
//!   4. `zero_leaf_matches_plain_diagnostic` — `FROM smelt.nonexistent` with no
//!      matching model emits `UndefinedModelRef` with no hint.
//!   5. `did_you_mean_helper_single_match` — unit-tests `leaf_did_you_mean`
//!      directly for the single-match path.
//!   6. `did_you_mean_helper_multi_match` — unit-tests `leaf_did_you_mean`
//!      directly for the multi-match path.
//!   7. `did_you_mean_helper_no_match` — unit-tests `leaf_did_you_mean` for the
//!      zero-match path.
//!
//! Path structure note: these tests omit a real `smelt.yml` on disk, so
//! `project_paths` returns an empty scan-root list and `file_path_tuple` does
//! NOT strip any directory prefix. For a file at `<root>/silver/events.sql`
//! the canonical path is `smelt.silver.events`. We therefore put model files
//! one directory below root (e.g. `silver/`, `bronze/`) so that the canonical
//! addresses read naturally as `smelt.silver.*` and `smelt.bronze.*`.

use std::path::PathBuf;

use smelt_db::{
    file_diagnostics, leaf_did_you_mean, Database, DiagnosticCode, SourceFile, Workspace,
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

/// `FROM smelt.events_parsed` (leaf-only path, missing `silver.` prefix) must
/// emit exactly one `UndefinedModelRef` diagnostic whose message contains a
/// "did you mean 'smelt.silver.events_parsed'?" hint.
///
/// Workspace layout (no smelt.yml → no scan-root stripping):
///   <root>/silver/raw_events.sql        → smelt.silver.raw_events
///   <root>/silver/events_parsed.sql     → smelt.silver.events_parsed
///   <root>/silver/downstream.sql        ← under test (references smelt.events_parsed)
#[test]
fn leaf_only_ref_is_undefined() {
    let root = PathBuf::from("/test/leaf_only");
    let raw_path = root.join("silver/raw_events.sql");
    let raw_src = "SELECT 1 AS event_id, 'click' AS event_type\n";

    let events_path = root.join("silver/events_parsed.sql");
    let events_src = "SELECT event_id FROM smelt.silver.raw_events\n";

    // The model under test references `smelt.events_parsed` — leaf-only (wrong).
    let caller_path = root.join("silver/downstream.sql");
    let caller_src = "SELECT event_id FROM smelt.events_parsed\n";

    let (db, ws, files) = build_db(
        root,
        &[
            (raw_path, raw_src),
            (events_path, events_src),
            (caller_path, caller_src),
        ],
    );
    let caller_file = files[2];
    let diags = file_diagnostics(&db, ws, caller_file);

    let undef: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UndefinedModelRef))
        .collect();
    assert_eq!(
        undef.len(),
        1,
        "expected exactly one UndefinedModelRef for leaf-only ref; got {diags:#?}"
    );
    let msg = &undef[0].message;
    assert!(
        msg.contains("did you mean 'smelt.silver.events_parsed'?"),
        "expected 'did you mean' hint in diagnostic message; got: {msg}"
    );
}

/// `FROM smelt.silver.events_parsed` (fully qualified) must produce no
/// diagnostics.
#[test]
fn correct_full_path_resolves() {
    let root = PathBuf::from("/test/full_path");
    let raw_path = root.join("silver/raw_events.sql");
    let raw_src = "SELECT 1 AS event_id, 'click' AS event_type\n";

    let events_path = root.join("silver/events_parsed.sql");
    let events_src = "SELECT event_id FROM smelt.silver.raw_events\n";

    let caller_path = root.join("silver/downstream.sql");
    let caller_src = "SELECT event_id FROM smelt.silver.events_parsed\n";

    let (db, ws, files) = build_db(
        root,
        &[
            (raw_path, raw_src),
            (events_path, events_src),
            (caller_path, caller_src),
        ],
    );
    let caller_file = files[2];
    let diags = file_diagnostics(&db, ws, caller_file);

    let undef: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UndefinedModelRef))
        .collect();
    assert!(
        undef.is_empty(),
        "fully qualified smelt.silver.events_parsed must produce no UndefinedModelRef; got {diags:#?}"
    );
}

/// Two models with the same leaf name in different directories. A ref that
/// resolves to neither (via a single-segment leaf path) should list both
/// candidates in the "did you mean" hint, sorted alphabetically.
///
/// Workspace layout:
///   <root>/silver/events.sql   → smelt.silver.events
///   <root>/bronze/events.sql   → smelt.bronze.events
///   <root>/gold/summary.sql    ← under test (references smelt.events — leaf-only)
#[test]
fn ambiguous_leaf_no_hint() {
    let root = PathBuf::from("/test/ambiguous");
    let silver_events_path = root.join("silver/events.sql");
    let silver_events_src = "SELECT 1 AS event_id\n";

    let bronze_events_path = root.join("bronze/events.sql");
    let bronze_events_src = "SELECT 2 AS event_id\n";

    // Third model references `smelt.events` — leaf-only, matches two candidates.
    let caller_path = root.join("gold/summary.sql");
    let caller_src = "SELECT event_id FROM smelt.events\n";

    let (db, ws, files) = build_db(
        root,
        &[
            (silver_events_path, silver_events_src),
            (bronze_events_path, bronze_events_src),
            (caller_path, caller_src),
        ],
    );
    let caller_file = files[2];
    let diags = file_diagnostics(&db, ws, caller_file);

    let undef: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UndefinedModelRef))
        .collect();
    assert_eq!(
        undef.len(),
        1,
        "expected exactly one UndefinedModelRef for ambiguous leaf ref; got {diags:#?}"
    );
    let msg = &undef[0].message;
    // Both candidates appear in the message, sorted alphabetically:
    // bronze.events < silver.events
    assert!(
        msg.contains("did you mean one of 'smelt.bronze.events', 'smelt.silver.events'?"),
        "expected both candidates in hint; got: {msg}"
    );
}

/// `FROM smelt.nonexistent` — leaf `nonexistent` matches no model — emits
/// `UndefinedModelRef` with no "did you mean" hint.
#[test]
fn zero_leaf_matches_plain_diagnostic() {
    let root = PathBuf::from("/test/no_match");
    let model_path = root.join("models/users.sql");
    let model_src = "SELECT 1 AS id\n";

    let caller_path = root.join("models/downstream.sql");
    let caller_src = "SELECT id FROM smelt.nonexistent\n";

    let (db, ws, files) = build_db(root, &[(model_path, model_src), (caller_path, caller_src)]);
    let caller_file = files[1];
    let diags = file_diagnostics(&db, ws, caller_file);

    let undef: Vec<_> = diags
        .iter()
        .filter(|d| d.code == Some(DiagnosticCode::UndefinedModelRef))
        .collect();
    assert_eq!(
        undef.len(),
        1,
        "expected exactly one UndefinedModelRef for unknown ref; got {diags:#?}"
    );
    let msg = &undef[0].message;
    assert!(
        !msg.contains("did you mean"),
        "no hint expected when leaf matches nothing; got: {msg}"
    );
}

/// Unit test for `leaf_did_you_mean` — single match returns exactly one
/// canonical path with the `smelt.` prefix.
///
/// File at `<root>/silver/events_parsed.sql` → canonical `smelt.silver.events_parsed`.
#[test]
fn did_you_mean_helper_single_match() {
    let root = PathBuf::from("/test/dym_single");
    let events_path = root.join("silver/events_parsed.sql");
    let events_src = "SELECT 1 AS event_id\n";

    let (db, ws, _files) = build_db(root, &[(events_path, events_src)]);
    let project = ws.projects(&db)[0];

    let candidates = leaf_did_you_mean(&db, ws, Some(project), "events_parsed");
    assert_eq!(
        candidates,
        vec!["smelt.silver.events_parsed".to_string()],
        "expected single candidate for leaf 'events_parsed'; got {candidates:?}"
    );
}

/// Unit test for `leaf_did_you_mean` — multiple matches returns all canonical
/// paths, sorted alphabetically.
#[test]
fn did_you_mean_helper_multi_match() {
    let root = PathBuf::from("/test/dym_multi");
    let bronze_path = root.join("bronze/events.sql");
    let bronze_src = "SELECT 1 AS event_id\n";
    let silver_path = root.join("silver/events.sql");
    let silver_src = "SELECT 2 AS event_id\n";

    let (db, ws, _files) = build_db(
        root,
        &[(bronze_path, bronze_src), (silver_path, silver_src)],
    );
    let project = ws.projects(&db)[0];

    let candidates = leaf_did_you_mean(&db, ws, Some(project), "events");
    assert_eq!(
        candidates,
        vec![
            "smelt.bronze.events".to_string(),
            "smelt.silver.events".to_string()
        ],
        "expected two candidates sorted alphabetically; got {candidates:?}"
    );
}

/// Unit test for `leaf_did_you_mean` — no match returns empty vec.
#[test]
fn did_you_mean_helper_no_match() {
    let root = PathBuf::from("/test/dym_none");
    let model_path = root.join("models/users.sql");
    let model_src = "SELECT 1 AS id\n";

    let (db, ws, _files) = build_db(root, &[(model_path, model_src)]);
    let project = ws.projects(&db)[0];

    let candidates = leaf_did_you_mean(&db, ws, Some(project), "nonexistent");
    assert!(
        candidates.is_empty(),
        "expected empty candidates for unknown leaf; got {candidates:?}"
    );
}
