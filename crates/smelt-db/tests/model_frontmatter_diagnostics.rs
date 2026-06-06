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

// ── BUG-026: week_start value-domain (LSP / file_diagnostics path) ──────────

/// `week_start: wednesday` on `granularity: week` must emit `MalformedTimeseries`
/// through `file_diagnostics` (the LSP/Salsa path), proving BUG-024's Error-severity
/// gate carries this new emission site.
#[test]
fn week_start_bad_value_emits_malformed_timeseries_in_file_diagnostics() {
    let root = PathBuf::from("/fake/model_fm_bad_week_start");
    let path = root.join("models").join("bad_week_start.sql");
    let src = "\
---
materialization: table
timeseries:
  event_time_column: event_timestamp
  partition_column: dt
  granularity: week
  week_start: wednesday
---
SELECT date_trunc('week', event_timestamp) AS dt, event_timestamp
FROM events
";
    let (db, ws, files) = build_db(root, SMELT_YML, &[(path, src)]);
    let file = files[0];

    let diags = diags_with_code(&db, ws, file, DiagnosticCode::MalformedTimeseries);
    assert!(
        diags
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error),
        "expected MalformedTimeseries Error for week_start: wednesday; got {:#?}",
        diags_for(&db, ws, file)
    );
}

/// `week_start: monday` and `week_start: sunday` must NOT emit any MalformedTimeseries
/// for the value-domain rule (other rules still apply).
#[test]
fn week_start_monday_and_sunday_do_not_emit_malformed_timeseries_for_value_domain() {
    for day in ["monday", "sunday"] {
        let root = PathBuf::from(format!("/fake/model_fm_week_start_{day}"));
        let path = root.join("models").join(format!("week_start_{day}.sql"));
        let src = format!(
            "\
---
materialization: table
timeseries:
  event_time_column: event_timestamp
  partition_column: dt
  granularity: week
  week_start: {day}
---
SELECT date_trunc('week', event_timestamp) AS dt, event_timestamp
FROM events
"
        );
        let (db, ws, files) = build_db(root, SMELT_YML, &[(path, src.as_str())]);
        let file = files[0];

        let diags = diags_with_code(&db, ws, file, DiagnosticCode::MalformedTimeseries);
        assert!(
            diags.is_empty(),
            "week_start: {day} must not emit MalformedTimeseries; got {:#?}",
            diags_for(&db, ws, file)
        );
    }
}
