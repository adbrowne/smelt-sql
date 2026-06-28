#![cfg(feature = "duckdb")]
//! End-to-end coverage for the incremental DELETE+INSERT contract
//! (`docs/specs/incremental_models.md`), exercised through the compiled
//! `smelt` binary rather than the library — the same outside-in discipline as
//! `smelt_shop_idempotency.rs`.
//!
//! The existing incremental tests cover the backend primitives
//! (`delete_partitions`, `insert_into_from_query`) and the refusal paths in
//! isolation. None drove a real incremental model end-to-end through
//! `smelt run --event-time-start/--event-time-end` and asserted the two
//! formal contracts the spec upholds:
//!
//!   * Constraint #7 — *idempotence under fixed input*: re-running the same
//!     `[start, end)` window converges to the same output-table state.
//!   * Constraint #6 — *per-partition equivalence with full refresh*: for every
//!     partition `p` in the run window, the incremental output filtered to `p`
//!     equals what a full refresh would produce for `p`.
//!
//! Plus the Surface guarantee that `--event-time-end` is **exclusive**.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// Path to the bundled hermetic example workspace.
fn workspace_template_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/incremental_idempotency")
}

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

/// Recursively copy `src` into `dst`.
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

/// Pre-populate `raw.pulse` with four days of events (2024-01-01 .. 2024-01-04),
/// three users, a deterministic per-(date,user) count. Mirrors a `load_raw.py`
/// step; the source has no `timeseries:` declaration, so it is read in full and
/// the framework's window filter is what scopes the run.
fn seed_raw_pulse(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch("CREATE SCHEMA IF NOT EXISTS raw;")?;
    // i in [0, 120): day = i % 4 (so each day gets 30 rows), user = i % 3.
    // Each (day, user) pair therefore gets exactly 10 rows.
    conn.execute_batch(
        "CREATE OR REPLACE TABLE raw.pulse AS
         SELECT
             TIMESTAMP '2024-01-01 00:00:00'
                 + INTERVAL (i % 4) DAY
                 + INTERVAL (i) MINUTE              AS event_ts,
             CAST(i % 3 AS INTEGER)                 AS user_id
         FROM range(120) AS t(i);",
    )?;
    Ok(())
}

/// Run `smelt run` for the given event-time window. Panics with captured
/// stdout/stderr on failure so the assertion message is informative.
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
            "`smelt run` ({label}) failed (exit {:?}); stderr:\n{}\nstdout:\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        );
    }
}

/// Open the DB, read the full output table, and close the connection before
/// returning. DuckDB is single-writer per file, so the test must not hold a
/// connection open while shelling out to a `smelt run` subprocess.
fn read_output_rows(db_path: &Path) -> Vec<(String, i32, i64)> {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    output_rows(&conn)
}

/// The full output table as a sorted `(date, user, count)` vector — the
/// canonical signature used for equality across runs.
fn output_rows(conn: &duckdb::Connection) -> Vec<(String, i32, i64)> {
    let mut stmt = conn
        .prepare(
            "SELECT CAST(event_date AS VARCHAR), user_id, event_count
             FROM main.daily_events ORDER BY event_date, user_id",
        )
        .expect("prepare output query");
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i32>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .expect("query output rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect output rows");
    rows
}

/// What a full refresh would produce for partition `p`, computed directly from
/// the raw source — the right-hand side of the per-partition equivalence
/// contract.
fn full_refresh_partition(conn: &duckdb::Connection, p: &str) -> Vec<(String, i32, i64)> {
    let mut stmt = conn
        .prepare(
            "SELECT CAST(CAST(event_ts AS DATE) AS VARCHAR), user_id, COUNT(*)
             FROM raw.pulse
             WHERE CAST(event_ts AS DATE) = ?
             GROUP BY 1, 2 ORDER BY 1, 2",
        )
        .expect("prepare full-refresh query");
    stmt.query_map([p], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i32>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })
    .expect("query full-refresh rows")
    .collect::<Result<Vec<_>, _>>()
    .expect("collect full-refresh rows")
}

#[test]
fn incremental_run_is_idempotent_and_partition_equivalent() {
    let tmp = TempDir::new().expect("create tempdir");
    let workspace = tmp.path().join("workspace");
    copy_dir_all(&workspace_template_dir(), &workspace).expect("copy example workspace");

    let db_path = tmp.path().join("incremental.duckdb");
    seed_raw_pulse(&db_path).expect("seed raw.pulse");

    // --- Run #1: window [2024-01-01, 2024-01-03). Exclusive end => Jan 1 + Jan 2
    //     only; Jan 3 and Jan 4 must NOT appear. ---
    run_window(&workspace, &db_path, "2024-01-01", "2024-01-03", "run #1");

    let after_run1 = read_output_rows(&db_path);
    let parts: Vec<String> = after_run1
        .iter()
        .map(|(d, _, _)| d.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    assert_eq!(
        parts,
        vec!["2024-01-01".to_string(), "2024-01-02".to_string()],
        "exclusive --event-time-end: window [01-01, 01-03) must contain only \
         Jan 1 and Jan 2, got {parts:?}"
    );
    // Each (day, user) pair has exactly 10 rows; 2 days x 3 users = 6 rows.
    assert_eq!(
        after_run1.len(),
        6,
        "expected 6 (date,user) groups in the window"
    );
    assert!(
        after_run1.iter().all(|(_, _, c)| *c == 10),
        "each (date,user) group has 10 events: {after_run1:?}"
    );

    // --- Constraint #7: idempotence. Re-run the identical window; state must
    //     not change. ---
    run_window(
        &workspace,
        &db_path,
        "2024-01-01",
        "2024-01-03",
        "run #2 (idempotency)",
    );
    let after_run2 = read_output_rows(&db_path);
    assert_eq!(
        after_run1, after_run2,
        "idempotence: re-running the same [start, end) must converge to the \
         same output-table state (DELETE+INSERT must not duplicate rows)"
    );

    // --- Constraint #6: per-partition equivalence with full refresh. ---
    {
        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        for p in &["2024-01-01", "2024-01-02"] {
            let incremental: Vec<_> = after_run2
                .iter()
                .filter(|(d, _, _)| d == p)
                .cloned()
                .collect();
            let full = full_refresh_partition(&conn, p);
            assert_eq!(
                incremental, full,
                "per-partition equivalence failed for {p}: incremental output \
                 must equal a full refresh filtered to the same partition"
            );
        }
    }

    // --- Extend the build: run the next window [2024-01-03, 2024-01-05).
    //     Earlier partitions must be untouched (committed chunks don't roll
    //     back), new partitions appended. ---
    run_window(
        &workspace,
        &db_path,
        "2024-01-03",
        "2024-01-05",
        "run #3 (extend)",
    );
    let after_run3 = read_output_rows(&db_path);
    let parts_after: Vec<String> = after_run3
        .iter()
        .map(|(d, _, _)| d.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    assert_eq!(
        parts_after,
        vec![
            "2024-01-01".to_string(),
            "2024-01-02".to_string(),
            "2024-01-03".to_string(),
            "2024-01-04".to_string(),
        ],
        "extending the window must add Jan 3 + Jan 4 while leaving Jan 1 + Jan 2 \
         intact, got {parts_after:?}"
    );
    // Jan 1 + Jan 2 rows must be byte-identical to run #1's output.
    let preserved: Vec<_> = after_run3
        .into_iter()
        .filter(|(d, _, _)| d == "2024-01-01" || d == "2024-01-02")
        .collect();
    assert_eq!(
        preserved, after_run1,
        "the earlier window's partitions must survive a disjoint later run \
         unchanged"
    );
}
