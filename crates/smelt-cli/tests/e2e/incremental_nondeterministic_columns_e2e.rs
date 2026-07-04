#![cfg(feature = "duckdb")]
//! Real-fixture coverage for `batched.nondeterministic_columns`
//! (docs/specs/batched_models.md §"Non-determinism and the equivalence
//! contract"; Constraint 13).
//!
//! Exercises `examples/incremental_nondeterministic_columns` — an incremental
//! model that stamps every row with `NOW()` into an `inserted_at` column
//! listed in `batched.nondeterministic_columns` — end-to-end through
//! `smelt run`. Asserts:
//!
//!   1. The build succeeds — `detect()` (the `smelt-logical` non-determinism
//!      flow/taint check) does not reject the model for a payload column the
//!      modeller explicitly opted in.
//!   2. The deterministic columns (`event_date`, `user_id`, `event_count`)
//!      match exactly between a full-window rebuild and a day-by-day
//!      incremental replay — the per-partition equivalence contract
//!      (Semantics § "Per-partition equivalence") holds up to the declared
//!      non-determinism, which is confined to `inserted_at`.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn workspace_template_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/incremental_nondeterministic_columns")
}

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

/// Seed the events source table. The source YAML is at
/// `models/sources/events.yml` with no `name:` override, so the smelt
/// compiler maps `smelt.sources.events` → `main.sources_events`.
///
/// Two days of data (2024-03-01, 2024-03-02), 3 users each, 2 events per user
/// per day.
fn seed_events(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch("CREATE SCHEMA IF NOT EXISTS main;")?;
    conn.execute_batch(
        r#"CREATE OR REPLACE TABLE main.sources_events AS
           SELECT
               CAST(i AS INTEGER)                                      AS event_id,
               TIMESTAMP '2024-03-01 08:00:00' + INTERVAL (i) MINUTE    AS event_ts,
               CAST('2024-03-01' AS DATE)                               AS event_date,
               CAST(i % 3 AS INTEGER)                                   AS user_id
           FROM range(6) AS t(i)
           UNION ALL
           SELECT
               CAST(100 + i AS INTEGER)                                 AS event_id,
               TIMESTAMP '2024-03-02 08:00:00' + INTERVAL (i) MINUTE    AS event_ts,
               CAST('2024-03-02' AS DATE)                               AS event_date,
               CAST(i % 3 AS INTEGER)                                   AS user_id
           FROM range(6) AS t(i);"#,
    )?;
    Ok(())
}

fn run_window(project_dir: &Path, db_path: &Path, start: &str, end: &str, label: &str) {
    let output = Command::new(smelt_bin())
        .args([
            "run",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--database",
            db_path.to_str().unwrap(),
            "--select",
            "daily_events",
            "--event-time-start",
            start,
            "--event-time-end",
            end,
        ])
        .env("RUST_LOG", "warn")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run` ({label}): {e}"));
    if !output.status.success() {
        panic!(
            "`smelt run` ({label}) failed (exit {:?});\nstderr:\n{}\nstdout:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        );
    }
}

/// Read the deterministic columns as (event_date, user_id, event_count),
/// deliberately excluding `inserted_at` — the opted-in non-deterministic
/// payload column is expected to vary between separate `smelt run`
/// invocations and is out of scope for the equivalence assertion.
fn read_deterministic_columns(db_path: &Path) -> Vec<(String, i64, i64)> {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    let mut stmt = conn
        .prepare(
            "SELECT CAST(event_date AS VARCHAR), user_id, event_count
             FROM main.daily_events
             ORDER BY event_date, user_id",
        )
        .expect("prepare query");
    stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })
    .expect("query rows")
    .collect::<Result<Vec<_>, _>>()
    .expect("collect rows")
}

/// Confirm `inserted_at` was actually populated (the NOW() projection made it
/// through the build, rather than being silently dropped or null).
fn count_non_null_inserted_at(db_path: &Path) -> i64 {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.query_row(
        "SELECT COUNT(*) FROM main.daily_events WHERE inserted_at IS NOT NULL",
        [],
        |row| row.get(0),
    )
    .expect("count non-null inserted_at")
}

#[test]
fn nondeterministic_columns_build_and_match_full_refresh_on_deterministic_columns() {
    let tmp = TempDir::new().expect("create tempdir");

    // ── Pipeline A: full-window single rebuild ────────────────────────────
    let workspace_a = tmp.path().join("workspace_a");
    copy_dir_all(&workspace_template_dir(), &workspace_a).expect("copy example workspace (A)");
    let db_a = tmp.path().join("full_refresh.duckdb");
    seed_events(&db_a).expect("seed events (A)");
    run_window(
        &workspace_a,
        &db_a,
        "2024-03-01",
        "2024-03-03",
        "pipeline-A (full)",
    );

    // ── Pipeline B: day-by-day incremental replay ─────────────────────────
    let workspace_b = tmp.path().join("workspace_b");
    copy_dir_all(&workspace_template_dir(), &workspace_b).expect("copy example workspace (B)");
    let db_b = tmp.path().join("incremental.duckdb");
    seed_events(&db_b).expect("seed events (B)");
    run_window(
        &workspace_b,
        &db_b,
        "2024-03-01",
        "2024-03-02",
        "pipeline-B day1",
    );
    run_window(
        &workspace_b,
        &db_b,
        "2024-03-02",
        "2024-03-03",
        "pipeline-B day2",
    );

    // ── The build itself succeeded (smelt run above would have panicked
    //    otherwise) — the non-determinism flow/taint check admitted the
    //    model because `inserted_at` is listed in
    //    `batched.nondeterministic_columns`. Confirm the payload column
    //    actually got populated on both pipelines.
    assert_eq!(
        count_non_null_inserted_at(&db_a),
        6,
        "pipeline A: inserted_at must be populated"
    );
    assert_eq!(
        count_non_null_inserted_at(&db_b),
        6,
        "pipeline B: inserted_at must be populated"
    );

    // ── Deterministic columns must match exactly, per partition ───────────
    let rows_a = read_deterministic_columns(&db_a);
    let rows_b = read_deterministic_columns(&db_b);

    assert!(!rows_a.is_empty(), "pipeline A produced no rows");
    assert_eq!(
        rows_a, rows_b,
        "deterministic columns (event_date, user_id, event_count) must match exactly between \
         full-refresh (A) and day-by-day incremental (B) — the per-partition equivalence \
         contract holds up to the declared non-determinism confined to `inserted_at`"
    );
}
