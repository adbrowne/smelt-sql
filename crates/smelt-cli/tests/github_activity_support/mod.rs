//! Shared replay harness for `examples/github_activity/` integration tests,
//! extracted from `github_activity_replay.rs`
//! (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/06-plan.md`) so
//! `github_activity_oracle.rs` can drive the same staging and day-load
//! machinery without duplicating it.
//!
//! Included via `mod github_activity_support;` in more than one test binary;
//! not every binary uses every function, so `dead_code` is allowed here
//! rather than at each call site.
#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_owned()
}

fn copy_dir_all(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap_or_else(|e| panic!("mkdir {dst:?}: {e}"));
    for entry in fs::read_dir(src).unwrap_or_else(|e| panic!("readdir {src:?}: {e}")) {
        let entry = entry.expect("dir entry");
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            // `target/` and `.smelt/` are both gitignored build/run state. A
            // fresh clone and CI have neither, so copying whatever a developer's
            // last local run happened to leave behind makes these tests depend
            // on untracked residue: a `.smelt/targets/dev/` carrying a posture
            // baseline from a differently-populated source turns the very next
            // staged run into a `SourceMutationProfileViolated` failure that
            // reproduces nowhere else.
            if matches!(
                from.file_name().and_then(|n| n.to_str()),
                Some("target") | Some(".smelt")
            ) {
                continue;
            }
            copy_dir_all(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap_or_else(|e| panic!("copy {from:?} -> {to:?}: {e}"));
        }
    }
}

/// Stage a fresh copy of `examples/github_activity/` under `tmp`, returning
/// (workspace_dir, db_path, sample_parquet_path).
pub fn stage_workspace(tmp: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let src = repo_root().join("examples/github_activity");
    let workspace = tmp.join("github_activity");
    copy_dir_all(&src, &workspace);
    let db = workspace.join("target/dev.duckdb");
    fs::create_dir_all(workspace.join("target")).expect("mkdir target");
    let sample = workspace.join("seeds/github_events_sample.parquet");
    (workspace, db, sample)
}

pub fn duckdb_exec(db: &Path, sql: &str) {
    let conn = duckdb::Connection::open(db).unwrap_or_else(|e| panic!("open {db:?}: {e}"));
    conn.execute_batch(sql)
        .unwrap_or_else(|e| panic!("exec failed: {e}\nSQL:\n{sql}"));
}

pub fn duckdb_scalar_i64(db: &Path, sql: &str) -> i64 {
    let conn = duckdb::Connection::open(db).unwrap_or_else(|e| panic!("open {db:?}: {e}"));
    conn.query_row(sql, [], |row| row.get(0))
        .unwrap_or_else(|e| panic!("query failed: {e}\nSQL:\n{sql}"))
}

pub fn create_empty_raw_table(db: &Path, sample: &Path) {
    duckdb_exec(
        db,
        &format!(
            "CREATE OR REPLACE TABLE main.sources_raw_github_events AS \
             SELECT * FROM read_parquet('{}') WHERE 1 = 0; \
             CREATE OR REPLACE TABLE main.sources_raw_github_events_arrival AS \
             SELECT *, CAST(NULL AS DATE) AS ingested_date \
             FROM read_parquet('{}') WHERE 1 = 0;",
            sample.display(),
            sample.display()
        ),
    );
}

/// Append day `day`'s real rows, plus a 2% redelivered slice of `day - 1`'s
/// rows, into both the event-time and arrival-partitioned relations, by
/// invoking `load_day.sh` — the same external-step program
/// `models/sources/raw/github_loader.yml` declares and `smelt run` invokes
/// on the run path. `prev` is unused: the script computes the previous day
/// itself via a SQL interval, so a day at the start of the fixture range
/// (with no D-1 rows in the sample) needs no special case; kept in the
/// signature so call sites (many, across `github_activity_replay.rs` and
/// `github_activity_oracle.rs`) are unchanged.
pub fn load_day(db: &Path, _sample: &Path, day: &str, _prev: Option<&str>) {
    if Command::new("duckdb").arg("--version").output().is_err() {
        panic!(
            "the `duckdb` CLI is not on PATH — required by \
             examples/github_activity/load_day.sh; provision it via \
             `mise run setup-duckdb` or the `setup-duckdb` GitHub Action"
        );
    }
    let workspace = db
        .parent()
        .and_then(Path::parent)
        .expect("db path is <workspace>/target/dev.duckdb");
    let script = workspace.join("load_day.sh");
    let out = Command::new("bash")
        .arg(&script)
        .args(["--date", day, "--database"])
        .arg(db)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn {script:?}: {e}"));
    if !out.status.success() {
        panic!(
            "load_day.sh --date {day} failed (exit {:?})\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }
}

pub fn smelt_run(workspace: &Path, start: &str, end: &str, extra_args: &[&str]) {
    let out = Command::new(smelt_bin())
        .args(["run", "--event-time-start", start, "--event-time-end", end])
        .args(extra_args)
        .current_dir(workspace)
        .env("RUST_LOG", "warn")
        // The staged `smelt.yml` declares a `databricks` target whose
        // `host`/`token` are `${SMELT_DBX_HOSTNAME}`/`${SMELT_DBX_TOKEN}`
        // (`docs/outcomes/20260912-databricks-dogfood-spine/phases/
        // 06-plan.md`). Config-load interpolation resolves every `${VAR}` in
        // the file regardless of which target is selected
        // (`docs/specs/smelt_yml.md` §Semantics item 8), so the default
        // `--target dev` run here needs dummy values present, not real ones.
        .env("SMELT_DBX_HOST", "unused-in-tests.cloud.databricks.com")
        .env("SMELT_DBX_HOSTNAME", "unused-in-tests.cloud.databricks.com")
        .env("DATABRICKS_HOST", "unused-in-tests.cloud.databricks.com")
        .env("SMELT_DBX_TOKEN", "unused-in-tests")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn smelt run: {e}"));
    if !out.status.success() {
        panic!(
            "smelt run [{start} .. {end}) failed (exit {:?})\nstdout:\n{}\nstderr:\n{}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }
}

/// Day range covered by the pinned fixture (`sample.sql`,
/// `docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md`).
pub const FIXTURE_DAYS: &[&str] = &[
    "2026-08-05",
    "2026-08-06",
    "2026-08-07",
    "2026-08-08",
    "2026-08-09",
    "2026-08-10",
    "2026-08-11",
    "2026-08-12",
    "2026-08-13",
    "2026-08-14",
    "2026-08-15",
    "2026-08-16",
    "2026-08-17",
    "2026-08-18",
    "2026-08-19",
    "2026-08-20",
    "2026-08-21",
    "2026-08-22",
    "2026-08-23",
    "2026-08-24",
    "2026-08-25",
    "2026-08-26",
    "2026-08-27",
    "2026-08-28",
    "2026-08-29",
    "2026-08-30",
    "2026-08-31",
    "2026-09-01",
    "2026-09-02",
    "2026-09-03",
];

pub fn day_after(day: &str) -> String {
    let d = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").expect("parse day");
    (d + chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string()
}

/// Drive the incremental replay leg: one `smelt run` per day, with **no**
/// pre-load — `models/sources/raw/github_loader.yml`'s declared external
/// step (`load_day.sh`) is what loads each day, invoked by `smelt run`
/// itself ahead of every model that reads either raw source. `db`/`sample`
/// are unused (the step creates the raw tables itself when absent); kept in
/// the signature so call sites are unchanged.
pub fn replay_days(workspace: &Path, _db: &Path, _sample: &Path, days: &[&str]) {
    for day in days {
        smelt_run(workspace, day, &day_after(day), &[]);
    }
}
