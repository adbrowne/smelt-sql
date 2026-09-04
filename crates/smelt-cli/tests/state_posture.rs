#![cfg(feature = "duckdb")]
//! `docs/outcomes/20260904-state-residency/outcome.md` phase 8:
//! `state.mode` honoured in `execute_project` — a per-posture `.smelt/`
//! write set, driven through the real `smelt` binary.
//!
//! Spec: `docs/specs/state.md` §"`state.mode` and what each posture
//! provides", §"The optionality rule".

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write_smelt_yml(dir: &Path, name: &str, state_mode: &str) {
    let yml = format!(
        "name: {name}\n\
         version: 1\n\
         paths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: table\n\
         state:\n  mode: {state_mode}\n"
    );
    std::fs::write(dir.join("smelt.yml"), yml).unwrap();
}

/// A partition-grain incremental model over a real `raw.events` table —
/// `save_intervals` only runs on the time-windowed incremental write path
/// (`execute.rs`), not a plain full-refresh table, so the posture tests
/// that assert `intervals.json` need this shape rather than a bare
/// `SELECT 1`.
const SQL_INCREMENTAL: &str = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_date, amount FROM raw.events
"#;

fn stage_workspace(tmp: &TempDir, name: &str, state_mode: &str) -> PathBuf {
    let root = tmp.path().join(name);
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    write_smelt_yml(&root, name, state_mode);
    std::fs::write(root.join("models/simple.sql"), SQL_INCREMENTAL).unwrap();

    let db_path = root.join("target/dev.duckdb");
    let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
    conn.execute_batch(
        "CREATE SCHEMA IF NOT EXISTS raw; \
         CREATE OR REPLACE TABLE raw.events AS SELECT * FROM (VALUES \
             (DATE '2026-01-01', 1.0) \
         ) AS t(event_date, amount);",
    )
    .expect("seed raw.events");
    drop(conn);
    root
}

fn run_smelt(project_dir: &Path, extra_args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(smelt_bin());
    cmd.args([
        "run",
        "--project-dir",
        project_dir.to_str().unwrap(),
        "--start",
        "2026-01-01",
        "--end",
        "2026-01-02",
    ])
    .args(extra_args)
    .env_remove("RUST_LOG");
    cmd.output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run`: {e}"))
}

fn walk_relative_paths(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                out.push(
                    path.strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    if dir.exists() {
        walk(dir, dir, &mut out);
    }
    out
}

/// `docs/specs/state.md` §"`state.mode` and what each posture provides":
/// `stateless` (the default) writes nothing — no `.smelt/` at all.
#[test]
fn stateless_run_creates_no_smelt_dir() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_workspace(&tmp, "posture_stateless", "stateless");

    let output = run_smelt(&ws, &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "run must succeed; stderr:\n{stderr}"
    );
    assert!(
        !ws.join(".smelt").exists(),
        "state.mode: stateless must leave no .smelt/ directory"
    );
}

/// `intervals` writes run manifests, reports, the interval ledger, and
/// (once a schema is deployed) schema snapshots — but never the
/// environments-only snapshot/environment store.
#[test]
fn intervals_run_writes_exactly_the_posture_set() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_workspace(&tmp, "posture_intervals", "intervals");

    let output = run_smelt(&ws, &[]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "run must succeed; stderr:\n{stderr}"
    );

    let smelt_dir = ws.join(".smelt");
    assert!(smelt_dir.exists(), "intervals posture must create .smelt/");

    let files = walk_relative_paths(&smelt_dir);
    assert!(
        files.iter().any(|f| f.starts_with("targets/dev/runs/")),
        "expected a run manifest under targets/dev/runs/, got {files:?}"
    );
    assert!(
        files.iter().any(|f| f == "targets/dev/intervals.json"),
        "expected the interval ledger, got {files:?}"
    );
    assert!(
        !files.iter().any(|f| f == "targets/dev/snapshots.json"),
        "intervals posture must never write the environments-only snapshot store, got {files:?}"
    );
}

/// `environments` is a superset of `intervals` — every file `intervals`
/// writes must still be written under `environments`.
#[test]
fn environments_run_adds_the_snapshot_store() {
    let tmp = TempDir::new().unwrap();
    let intervals_ws = stage_workspace(&tmp, "posture_intervals_cmp", "intervals");
    let env_ws = stage_workspace(&tmp, "posture_environments", "environments");

    for ws in [&intervals_ws, &env_ws] {
        let output = run_smelt(ws, &[]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "run must succeed; stderr:\n{stderr}"
        );
    }

    // Normalize away the run-id-stamped filename (`runs/<id>.json`,
    // `reports/<id>.json`) so this compares artifact *kinds*, not the
    // incidentally-different run ids each invocation generates.
    fn normalize(paths: Vec<String>) -> std::collections::BTreeSet<String> {
        paths
            .into_iter()
            .map(|p| {
                if let Some(dir) = p
                    .strip_suffix(".json")
                    .and_then(|p| p.rsplit_once('/'))
                    .map(|(dir, _)| dir)
                {
                    if dir.ends_with("/runs") || dir.ends_with("/reports") {
                        return format!("{dir}/*.json");
                    }
                }
                p
            })
            .collect()
    }

    let intervals_files = normalize(walk_relative_paths(&intervals_ws.join(".smelt")));
    let env_files = normalize(walk_relative_paths(&env_ws.join(".smelt")));
    for f in &intervals_files {
        assert!(
            env_files.contains(f),
            "environments posture must write every artifact kind intervals writes; \
             missing {f} from {env_files:?}"
        );
    }
}

/// `docs/specs/state.md` §"The optionality rule": `--resume` under
/// `stateless` refuses by naming the posture, not the generic "no
/// partially-failed run" message.
#[test]
fn resume_under_stateless_refuses_naming_the_posture() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_workspace(&tmp, "posture_resume_stateless", "stateless");

    let output = run_smelt(&ws, &["--resume"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "--resume under stateless must fail"
    );
    assert!(
        stderr.contains("state.mode: stateless"),
        "the refusal must name the posture; got stderr:\n{stderr}"
    );
}

/// `smelt history` on a stateless project must say the posture is why
/// there is no history, not print an empty result indistinguishable from
/// "no runs yet".
#[test]
fn history_under_stateless_names_the_posture() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_workspace(&tmp, "posture_history_stateless", "stateless");

    let run_output = run_smelt(&ws, &[]);
    assert!(run_output.status.success());

    let mut cmd = Command::new(smelt_bin());
    cmd.args(["history", "--project-dir", ws.to_str().unwrap()])
        .env_remove("RUST_LOG");
    let output = cmd.output().expect("failed to spawn `smelt history`");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("state.mode: stateless"),
        "history must name the posture when no run history exists under \
         stateless; got stdout:\n{stdout}"
    );
}
