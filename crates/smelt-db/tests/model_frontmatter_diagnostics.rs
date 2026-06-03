//! U3 regression — model-file frontmatter parse errors must surface through
//! `file_diagnostics` instead of being silently swallowed.
//!
//! Before U3 the two sites where errors were swallowed were:
//!   - `smelt-core/src/discovery.rs` (~line 264): `Err(e) => { eprintln!(...); None }`
//!   - `smelt-db/src/lib.rs` (~line 1175): `extract_file_metadata` returning `Err` when
//!     ModelMetadata had `deny_unknown_fields` and encountered a function key.
//!
//! These tests pin the corrected behaviour per the Unified Frontmatter Rule:
//!   - Unknown top-level key   → FrontmatterParseError (Error)
//!   - Inapplicable key        → FrontmatterParseError (Warning), block retained
//!   - Bad timeseries sub-key  → MalformedTimeseries (Error)
//!   - Bad timeseries value    → MalformedTimeseries (Error)

use std::path::PathBuf;

use smelt_db::{
    file_diagnostics, Database, Diagnostic, DiagnosticCode, DiagnosticSeverity, SourceFile,
    Workspace,
};

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

const SMELT_YML: &str = "name: model_frontmatter_test
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    schema: main
default_materialization: view
";

fn diags_for(db: &Database, ws: Workspace, file: SourceFile) -> Vec<Diagnostic> {
    file_diagnostics(db, ws, file)
}

fn diags_with_code(
    db: &Database,
    ws: Workspace,
    file: SourceFile,
    code: DiagnosticCode,
) -> Vec<Diagnostic> {
    file_diagnostics(db, ws, file)
        .into_iter()
        .filter(|d| d.code == Some(code))
        .collect()
}

// ── BUG-023: timeseries.granularity bad value ────────────────────────────────

/// A model with `granularity: fortnight` in its timeseries block must emit a
/// `MalformedTimeseries` error instead of silently reverting to VIEW.
/// Previously `serde_yaml` rejected the unknown enum variant and the error
/// was swallowed at the discovery and smelt-db sites.
#[test]
fn timeseries_bad_granularity_emits_malformed_timeseries() {
    let root = PathBuf::from("/fake/model_fm_bad_granularity");
    let path = root.join("models").join("bad_gran.sql");
    let src = "\
---
materialization: table
timeseries:
  event_time_column: event_time
  partition_column: date
  granularity: fortnight
---
SELECT event_time, date FROM events
";
    let (db, ws, files) = build_db(root, SMELT_YML, &[(path, src)]);
    let file = files[0];

    let diags = diags_with_code(&db, ws, file, DiagnosticCode::MalformedTimeseries);
    assert!(
        diags
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error),
        "expected at least one MalformedTimeseries Error for bad granularity; got {:#?}",
        diags_for(&db, ws, file)
    );
}

// ── BUG-025 (extended): unknown timeseries sub-key ───────────────────────────

/// An unknown sub-key in the `timeseries:` block (`partition_columm` typo)
/// must emit `MalformedTimeseries`.
/// `TimeseriesConfig` has `deny_unknown_fields` (added in U1), so serde rejects
/// the typo. Previously that error was swallowed; now it surfaces.
#[test]
fn timeseries_unknown_subkey_emits_malformed_timeseries() {
    let root = PathBuf::from("/fake/model_fm_bad_subkey");
    let path = root.join("models").join("bad_subkey.sql");
    let src = "\
---
materialization: table
timeseries:
  event_time_column: event_time
  partition_columm: date
  granularity: day
---
SELECT event_time, date FROM events
";
    let (db, ws, files) = build_db(root, SMELT_YML, &[(path, src)]);
    let file = files[0];

    let diags = diags_with_code(&db, ws, file, DiagnosticCode::MalformedTimeseries);
    assert!(
        diags
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error),
        "expected at least one MalformedTimeseries Error for unknown timeseries sub-key; got {:#?}",
        diags_for(&db, ws, file)
    );
}

// ── BUG-016: unknown top-level key ───────────────────────────────────────────

/// An unknown top-level key (typo like `mateializaton`) must emit a
/// `FrontmatterParseError` of Error severity.
/// Previously `ModelMetadata`'s `deny_unknown_fields` caused a serde Err that
/// was swallowed; the model silently defaulted to `view` with no diagnostic.
#[test]
fn unknown_toplevel_key_emits_frontmatter_parse_error() {
    let root = PathBuf::from("/fake/model_fm_unknown_key");
    let path = root.join("models").join("unknown_key.sql");
    let src = "\
---
mateializaton: table
---
SELECT 1 AS val
";
    let (db, ws, files) = build_db(root, SMELT_YML, &[(path, src)]);
    let file = files[0];

    let diags = diags_with_code(&db, ws, file, DiagnosticCode::FrontmatterParseError);
    assert!(
        diags
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error),
        "expected at least one FrontmatterParseError Error for unknown top-level key; got {:#?}",
        diags_for(&db, ws, file)
    );
}

// ── BUG-016: function key on model → Warning + materialization retained ──────

/// A model with `materialization: table` + `deterministic: true` (a function
/// key, not applicable to models) must emit exactly one Warning-severity
/// `FrontmatterParseError` and no Error-severity FrontmatterParseError.
/// Previously the `deny_unknown_fields` on `ModelMetadata` rejected the whole
/// block, metadata defaulted to None, and materialization silently became VIEW.
#[test]
fn function_key_on_model_emits_warning_not_error() {
    let root = PathBuf::from("/fake/model_fm_function_key");
    let path = root.join("models").join("fn_key_on_model.sql");
    let src = "\
---
materialization: table
deterministic: true
---
SELECT 1 AS val
";
    let (db, ws, files) = build_db(root, SMELT_YML, &[(path, src)]);
    let file = files[0];

    let diags = diags_with_code(&db, ws, file, DiagnosticCode::FrontmatterParseError);

    // Must have at least one Warning (for the inapplicable `deterministic` key).
    assert!(
        diags
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Warning),
        "expected a Warning for inapplicable key `deterministic` on model; got {:#?}",
        diags_for(&db, ws, file)
    );

    // Must NOT have an Error from the frontmatter block itself being rejected.
    assert!(
        !diags
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error),
        "must not emit a FrontmatterParseError Error — block should be retained; got {:#?}",
        diags_for(&db, ws, file)
    );
}
