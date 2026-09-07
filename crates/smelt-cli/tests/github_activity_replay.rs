#![cfg(feature = "duckdb")]
//! Replay harness for `examples/github_activity/`
//! (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/02-plan.md`).
//!
//! Asserts, over a short (7-day) slice of the committed Parquet fixture, the
//! properties the phase 2 plan's "Tests (red first)" section names:
//!
//!   - the loader's deliberate previous-day redelivery actually lands
//!     duplicate physical rows in `raw.github_events`, and
//!     `silver.events_deduped` emits exactly `count(DISTINCT id)` rows;
//!   - the declared `key_recurrence` bound on `raw.github_events` is
//!     genuinely checked, not decorative: a duplicate pair that violates it
//!     fails the run transactionally (`KeyedRecurrenceBoundViolated`) rather
//!     than silently mis-counting — the negative control the plan asks for.
//!     (`silver.events_deduped`'s dedup itself is a keyed `MERGE`, which is
//!     idempotent regardless of window width, so there is no SQL-derived
//!     lookback to narrow the way `events_parsed`'s arrival-based filter has
//!     in `examples/web_analytics/` — see that model's own comment for why.)
//!   - a session spanning a UTC midnight stays one session, not two;
//!   - full-refresh and the day-by-day incremental replay agree on every
//!     model, over the full 30-day fixture range.
//!
//! 7 days keeps CI runtime bounded; the full-refresh equivalence test uses
//! the entire 30-day fixture since that is what the outcome's criterion 7
//! (DuckDB half) actually promises.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
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
            if from.file_name().and_then(|n| n.to_str()) == Some("target") {
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
fn stage_workspace(tmp: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let src = repo_root().join("examples/github_activity");
    let workspace = tmp.join("github_activity");
    copy_dir_all(&src, &workspace);
    let db = workspace.join("target/dev.duckdb");
    fs::create_dir_all(workspace.join("target")).expect("mkdir target");
    let sample = workspace.join("seeds/github_events_sample.parquet");
    (workspace, db, sample)
}

fn duckdb_exec(db: &Path, sql: &str) {
    let conn = duckdb::Connection::open(db).unwrap_or_else(|e| panic!("open {db:?}: {e}"));
    conn.execute_batch(sql)
        .unwrap_or_else(|e| panic!("exec failed: {e}\nSQL:\n{sql}"));
}

fn duckdb_scalar_i64(db: &Path, sql: &str) -> i64 {
    let conn = duckdb::Connection::open(db).unwrap_or_else(|e| panic!("open {db:?}: {e}"));
    conn.query_row(sql, [], |row| row.get(0))
        .unwrap_or_else(|e| panic!("query failed: {e}\nSQL:\n{sql}"))
}

fn create_empty_raw_table(db: &Path, sample: &Path) {
    duckdb_exec(
        db,
        &format!(
            "CREATE OR REPLACE TABLE main.sources_raw_github_events AS \
             SELECT * FROM read_parquet('{}') WHERE 1 = 0;",
            sample.display()
        ),
    );
}

/// Append day `day`'s real rows, plus (unless `first_day`) a 2% redelivered
/// slice of `day - 1`'s rows — mirrors `run_incremental.py::load_day`.
fn load_day(db: &Path, sample: &Path, day: &str, prev: Option<&str>) {
    let redelivery = match prev {
        Some(p) => format!(
            "UNION ALL SELECT * FROM read_parquet('{}') \
             WHERE CAST(created_at AS DATE) = DATE '{p}' \
             AND MOD(CAST(id AS BIGINT), 50) = 0",
            sample.display()
        ),
        None => String::new(),
    };
    duckdb_exec(
        db,
        &format!(
            "INSERT INTO main.sources_raw_github_events \
             SELECT * FROM read_parquet('{}') \
             WHERE CAST(created_at AS DATE) = DATE '{day}' {redelivery};",
            sample.display()
        ),
    );
}

fn smelt_run(workspace: &Path, start: &str, end: &str, extra_args: &[&str]) {
    let out = Command::new(smelt_bin())
        .args(["run", "--event-time-start", start, "--event-time-end", end])
        .args(extra_args)
        .current_dir(workspace)
        .env("RUST_LOG", "warn")
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

fn smelt_run_expect_failure(workspace: &Path, start: &str, end: &str) -> String {
    let out = Command::new(smelt_bin())
        .args(["run", "--event-time-start", start, "--event-time-end", end])
        .current_dir(workspace)
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn smelt run: {e}"));
    assert!(
        !out.status.success(),
        "expected smelt run [{start} .. {end}) to fail, but it succeeded\nstdout:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Day range covered by the pinned fixture (`sample.sql`,
/// `docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md`).
const FIXTURE_DAYS: &[&str] = &[
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

fn day_after(day: &str) -> String {
    let d = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").expect("parse day");
    (d + chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string()
}

fn replay_days(workspace: &Path, db: &Path, sample: &Path, days: &[&str]) {
    create_empty_raw_table(db, sample);
    let mut prev: Option<&str> = None;
    for day in days {
        load_day(db, sample, day, prev);
        smelt_run(workspace, day, &day_after(day), &[]);
        prev = Some(day);
    }
}

/// The loader's redelivery lands physical duplicates, and the keyed dedup
/// collapses them to exactly `count(DISTINCT id)` — the plan's headline
/// assertion (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/
/// 02-plan.md` §"Tests (red first)").
#[test]
fn redelivered_rows_land_and_dedup_collapses_to_distinct_ids() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    replay_days(&workspace, &db, &sample, &FIXTURE_DAYS[0..7]);

    let raw_rows = duckdb_scalar_i64(&db, "SELECT count(*) FROM main.sources_raw_github_events");
    let distinct_ids = duckdb_scalar_i64(
        &db,
        "SELECT count(DISTINCT id) FROM main.sources_raw_github_events",
    );
    let deduped_rows = duckdb_scalar_i64(&db, "SELECT count(*) FROM main.silver_events_deduped");

    assert!(
        raw_rows > distinct_ids,
        "expected the redelivery to produce physical duplicates in raw \
         (raw_rows={raw_rows}, distinct_ids={distinct_ids}) — a fixture that \
         never redelivers never exercises dedup"
    );
    assert_eq!(
        deduped_rows, distinct_ids,
        "silver.events_deduped must emit exactly count(DISTINCT id) rows \
         (raw_rows={raw_rows}, distinct_ids={distinct_ids}, deduped_rows={deduped_rows})"
    );
}

/// Negative control: `raw.github_events` declares `key_recurrence: {key:
/// [id], window: '0 days'}` because a genuine redelivered duplicate always
/// shares its original's `created_at` exactly. A duplicate pair that
/// violates that bound (here: same `id`, `created_at` shifted by a day) must
/// fail the run transactionally rather than silently mis-dedup — proving the
/// declared bound is checked, not decorative
/// (`docs/specs/incremental_shapes.md` §"Key temporal locality").
#[test]
fn recurrence_bound_violation_fails_the_run() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    create_empty_raw_table(&db, &sample);
    load_day(&db, &sample, "2026-08-05", None);
    smelt_run(&workspace, "2026-08-05", "2026-08-06", &[]);

    // Inject a corrupt "redelivery": same id as a real 2026-08-05 row, but
    // shifted a day later on the event-time axis — violating the declared
    // zero-width recurrence bound.
    duckdb_exec(
        &db,
        &format!(
            "INSERT INTO main.sources_raw_github_events \
             SELECT id, type, created_at + INTERVAL '1 day', actor_id, actor_login, \
                    repo_id, repo_name, org_id, public \
             FROM read_parquet('{}') \
             WHERE CAST(created_at AS DATE) = DATE '2026-08-05' \
             LIMIT 1;",
            sample.display()
        ),
    );
    load_day(&db, &sample, "2026-08-06", None);

    let output = smelt_run_expect_failure(&workspace, "2026-08-06", "2026-08-07");
    assert!(
        output.contains("KeyedRecurrenceBoundViolated") || output.contains("RecurrenceBound"),
        "expected a recurrence-bound violation diagnostic, got:\n{output}"
    );
}

/// A session whose events straddle a UTC midnight stays one session (the
/// clock-anchored cut, `functions/sessionize.sql`), not two.
#[test]
fn session_spanning_midnight_stays_one_session() {
    let tmp = TempDir::new().expect("tempdir");
    let (workspace, db, sample) = stage_workspace(tmp.path());
    replay_days(&workspace, &db, &sample, &FIXTURE_DAYS[0..3]);

    // Any actor with events on both sides of a midnight, less than 30
    // minutes apart, must appear as exactly one session spanning that
    // boundary rather than two sessions ending/starting at the boundary.
    let violations = duckdb_scalar_i64(
        &db,
        "SELECT count(*)
         FROM main.silver_actor_sessions s1
         JOIN main.silver_actor_sessions s2
           ON s1.actor_id = s2.actor_id
          AND s1.session_start_ts < s2.session_start_ts
          AND epoch_us(s2.session_start) - epoch_us(s1.session_end) < 30 * 60 * 1000000",
    );
    assert_eq!(
        violations, 0,
        "found adjacent same-actor sessions within the 30-minute gap that should have merged \
         into one clock-anchored session"
    );
}

/// Full-refresh and the day-by-day incremental replay agree on every model,
/// over the full 30-day fixture — criterion 7's DuckDB half
/// (`docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md`).
#[test]
fn full_refresh_matches_incremental_replay() {
    let tmp = TempDir::new().expect("tempdir");

    let (incr_workspace, incr_db, incr_sample) = stage_workspace(&tmp.path().join("incremental"));
    replay_days(&incr_workspace, &incr_db, &incr_sample, FIXTURE_DAYS);

    let (full_workspace, full_db, full_sample) = stage_workspace(&tmp.path().join("full"));
    create_empty_raw_table(&full_db, &full_sample);
    duckdb_exec(
        &full_db,
        &format!(
            "INSERT INTO main.sources_raw_github_events SELECT * FROM read_parquet('{}');",
            full_sample.display()
        ),
    );
    smelt_run(
        &full_workspace,
        "2026-08-05",
        "2026-09-04",
        &["--full-refresh"],
    );

    for table in [
        "silver_events_deduped",
        "silver_actor_sessions",
        "marts_daily_active_contributors",
    ] {
        let incr_count = duckdb_scalar_i64(&incr_db, &format!("SELECT count(*) FROM main.{table}"));
        let full_count = duckdb_scalar_i64(&full_db, &format!("SELECT count(*) FROM main.{table}"));
        assert_eq!(
            incr_count, full_count,
            "row count mismatch for {table}: incremental={incr_count} full_refresh={full_count}"
        );
    }

    let incr_deduped = duckdb_scalar_i64(
        &incr_db,
        "SELECT count(DISTINCT id) FROM main.silver_events_deduped",
    );
    let full_deduped = duckdb_scalar_i64(
        &full_db,
        "SELECT count(DISTINCT id) FROM main.silver_events_deduped",
    );
    assert_eq!(incr_deduped, full_deduped);
    assert_eq!(
        incr_deduped, 64_313,
        "expected the full fixture to dedup to the measured 64,313 distinct ids \
         (docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md)"
    );
}
