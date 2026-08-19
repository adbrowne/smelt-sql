#![cfg(feature = "duckdb")]
//! Diagnostic-parity gate: `smelt build` must refuse to compile/execute any
//! workspace whose analyzer (the same surface the LSP publishes —
//! `file_diagnostics` + `check_type_diagnostics`) reports an `Error`-severity
//! diagnostic, naming the offending `DiagnosticCode`.
//!
//! Spec: `docs/specs/architecture.md` §"Diagnostic parity rule (analysis ↔
//! build)". This is the end-to-end (build-path) half of the parity guarantee;
//! `example_diagnostics.rs` covers the analysis-path half.
//!
//! Each test below is RED before the shared gate lands: the malformed-timeseries
//! and clean cases mis-build at exit 0, and the CTE-cycle / config-loader cases
//! fail for the *wrong* reason (a downstream DuckDB binder error / a misleading
//! "undefined ref" dependency error) without naming the analyzer's code.

use std::path::{Path, PathBuf};
use std::process::Command;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn examples_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
}

/// Write `files` (relative path → contents) into a fresh tempdir and return it.
fn make_workspace(files: &[(&str, &str)]) -> tempfile::TempDir {
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    for (rel, contents) in files {
        let path = tmp.path().join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create parent dir");
        }
        std::fs::write(&path, contents).expect("write fixture file");
    }
    tmp
}

/// Recursively copy `src` into `dst`, skipping the `target/` build tree.
fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == "target" {
            continue;
        }
        let target = dst.join(&name);
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

/// `smelt build` against `project_dir`, returning (success, combined stdout+stderr).
fn run_build(project_dir: &Path) -> (bool, String) {
    let out = Command::new(smelt_bin())
        .arg("build")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"));
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), combined)
}

const DEV_SMELT_YML: &str = "\
name: gate_fixture
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main
default_materialization: view
";

/// BUG-024: a structurally-malformed-but-parseable `timeseries:` block
/// (`week_start` on `granularity: day`) emits `MalformedTimeseries` (Error) in
/// `file_diagnostics`, yet today `smelt build` materialises it at exit 0.
#[test]
fn gate_blocks_malformed_timeseries() {
    let model = "\
---
materialization: table
timeseries:
  partition_column: event_date
  event_time_column: event_timestamp
  granularity: day
  week_start: monday
---
SELECT
    date_trunc('day', event_timestamp) AS event_date,
    event_timestamp
FROM (SELECT TIMESTAMP '2024-01-01' AS event_timestamp) t
";
    let ws = make_workspace(&[("smelt.yml", DEV_SMELT_YML), ("models/bad_ts.sql", model)]);
    let (ok, output) = run_build(ws.path());
    assert!(
        !ok,
        "expected `smelt build` to FAIL on a MalformedTimeseries model, but it succeeded.\n{output}"
    );
    assert!(
        output.contains("MalformedTimeseries"),
        "build failed but did not name the MalformedTimeseries diagnostic.\n{output}"
    );
}

/// BUG-019: a `smelt.define` whose body contains a cyclic CTE emits `CteCycle`
/// (Error) in `file_diagnostics`, yet today the build splices it verbatim and
/// fails downstream in DuckDB (or builds clean) without naming `CteCycle`.
#[test]
fn gate_blocks_cte_cycle() {
    let function = "\
smelt.define bad_cte(
    source: TableExpr
) -> TableExpr AS (
    WITH a AS (SELECT * FROM b),
         b AS (SELECT * FROM a)
    SELECT * FROM a
)
";
    let ws = make_workspace(&[
        ("smelt.yml", DEV_SMELT_YML),
        ("functions/bad.sql", function),
        ("models/noop.sql", "SELECT 1 AS x\n"),
    ]);
    let (ok, output) = run_build(ws.path());
    assert!(
        !ok,
        "expected `smelt build` to FAIL on a cyclic-CTE function, but it succeeded.\n{output}"
    );
    assert!(
        output.contains("CteCycle"),
        "build failed but did not name the CteCycle diagnostic.\n{output}"
    );
}

/// BUG-015: a config-loader schema violation (`configs/incomplete.yaml` missing
/// a required field) emits `ConfigLoaderRequiredFieldMissing` (Error) in
/// `file_diagnostics`, yet today the build fails with a misleading "undefined
/// model/source 'config.load_yaml'" dependency error that never names the code.
#[test]
fn gate_blocks_config_loader_error() {
    let src = examples_root().join("meta_config_broken_config_loader_required_field_missing");
    assert!(
        src.join("smelt.yml").is_file(),
        "fixture missing: {}",
        src.display()
    );
    let tmp = tempfile::TempDir::new().expect("create tempdir");
    let dest = tmp.path().join("ws");
    copy_dir(&src, &dest);

    let (ok, output) = run_build(&dest);
    assert!(
        !ok,
        "expected `smelt build` to FAIL on a config-loader schema violation.\n{output}"
    );
    assert!(
        output.contains("ConfigLoaderRequiredFieldMissing"),
        "build failed but did not name the ConfigLoaderRequiredFieldMissing diagnostic.\n{output}"
    );
}

/// BUG-026: `week_start: wednesday` on `granularity: week` is outside the
/// valid domain {monday, sunday} → `MalformedTimeseries` (Error). `smelt build`
/// must refuse to compile it and name the diagnostic.
#[test]
fn gate_blocks_invalid_week_start_value() {
    let model = "\
---
materialization: table
timeseries:
  partition_column: dt
  event_time_column: event_timestamp
  granularity: week
  week_start: wednesday
---
SELECT date_trunc('week', event_timestamp) AS dt, event_timestamp
FROM (SELECT TIMESTAMP '2024-01-01' AS event_timestamp) t
";
    let ws = make_workspace(&[("smelt.yml", DEV_SMELT_YML), ("models/bad_ws.sql", model)]);
    let (ok, output) = run_build(ws.path());
    assert!(
        !ok,
        "expected `smelt build` to FAIL on week_start: wednesday, but it succeeded.\n{output}"
    );
    assert!(
        output.contains("MalformedTimeseries"),
        "build failed but did not name the MalformedTimeseries diagnostic.\n{output}"
    );
}

/// Negative control: a clean workspace must still build (the gate must not
/// over-block).
#[test]
fn gate_allows_clean_workspace() {
    let ws = make_workspace(&[
        ("smelt.yml", DEV_SMELT_YML),
        ("models/ok.sql", "SELECT 1 AS x, 'hi' AS label\n"),
    ]);
    let (ok, output) = run_build(ws.path());
    assert!(
        ok,
        "expected a clean workspace to build, but `smelt build` failed.\n{output}"
    );
}

/// A user-written projection alias claiming the reserved `_smelt_` prefix
/// emits `ReservedProjectionAliasPrefix` (Error) in `file_diagnostics` —
/// `smelt build` must refuse and name the code, the same diagnostic the
/// LSP would publish for the identical model
/// (`docs/specs/multi_backend.md` §"Output-schema type conformance").
#[test]
fn gate_blocks_reserved_smelt_alias_prefix() {
    let ws = make_workspace(&[
        ("smelt.yml", DEV_SMELT_YML),
        ("models/bad_alias.sql", "SELECT 1 AS _smelt_foo\n"),
    ]);
    let (ok, output) = run_build(ws.path());
    assert!(
        !ok,
        "expected `smelt build` to FAIL on a user alias claiming the reserved \
         `_smelt_` prefix, but it succeeded.\n{output}"
    );
    assert!(
        output.contains("ReservedProjectionAliasPrefix"),
        "build failed but did not name the ReservedProjectionAliasPrefix diagnostic.\n{output}"
    );
}
