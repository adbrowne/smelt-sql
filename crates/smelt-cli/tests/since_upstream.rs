#![cfg(feature = "duckdb")]
//! MP15 (`docs/plans/20260707-maintenance-plan-impl.md`): `smelt run
//! --since-upstream` — forward propagation from caller-declared per-source
//! deltas (`maintenance_plan.md` §CLI, §"The graph layer"). Per the ratified
//! decision (2026-07-10, "Blocked phases"), the delta source is explicit
//! (`--source <address> --landed <start>..<end>`, repeatable) — no
//! `smelt-state` watermark, no automatic recorded-state diffing.
//!
//! Fixture: `silver` reads two CLOCKED append-only sources (`bronze`,
//! `aux`), joined via an explicit derivable window predicate (`aux.d2
//! BETWEEN bronze.d - 1 day AND bronze.d + 1 day`) — the same real-clamp
//! shape `examples/timeseries/models/daily_events_status.sql` exercises for
//! a genuine, derived `ScanClamp` (not a hand-typed number, not the
//! accepted-full-scan corner). Both sources are pre-populated directly
//! (sources have no CSV-seed mechanism) via `duckdb::Connection` at the
//! table names `execute_project` itself resolves them to
//! (`main.sources_<address>`).

use std::path::{Path, PathBuf};
use std::process::Command;

use duckdb::Connection;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// Stage a workspace with `silver` reading two clocked sources
/// (`sources.bronze`, `sources.aux`), joined via an explicit ±1-day window,
/// under `parent/proj`. Returns the project dir; the target DuckDB file is
/// `target/dev.duckdb`.
fn stage_workspace(parent: &Path) -> PathBuf {
    let root = parent.join("proj");
    write(
        &root,
        "smelt.yml",
        "name: since_upstream_ws\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\n",
    );
    write(
        &root,
        "models/sources/bronze.yml",
        "description: bronze\ncolumns:\n- name: id\n  type: INTEGER\n- name: d\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    );
    write(
        &root,
        "models/sources/aux.yml",
        "description: aux\ncolumns:\n- name: id\n  type: INTEGER\n- name: d2\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d2\n  event_time_column: d2\n  granularity: day\n",
    );
    write(
        &root,
        "models/silver.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT b.id, b.d, a.id AS aux_id\nFROM smelt.sources.bronze b\n\
         JOIN smelt.sources.aux a\n  ON a.id = b.id\n\
         AND a.d2 BETWEEN b.d - INTERVAL '1 day' AND b.d + INTERVAL '1 day'\n",
    );
    std::fs::create_dir_all(root.join("target")).unwrap();
    root
}

/// Pre-populate `main.sources_bronze` / `main.sources_aux` with 10 days of
/// data (2026-01-01 .. 2026-01-10), id == day-of-month on both, so every
/// bronze row has a same-day matching aux row.
fn seed_sources(db_path: &Path) {
    let conn = Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        "CREATE SCHEMA IF NOT EXISTS main;\n\
         CREATE TABLE main.sources_bronze (id INTEGER, d DATE);\n\
         CREATE TABLE main.sources_aux (id INTEGER, d2 DATE);\n\
         INSERT INTO main.sources_bronze \
           SELECT i, DATE '2026-01-01' + CAST(i - 1 AS INTEGER) FROM range(1, 11) t(i);\n\
         INSERT INTO main.sources_aux \
           SELECT i, DATE '2026-01-01' + CAST(i - 1 AS INTEGER) FROM range(1, 11) t(i);\n",
    )
    .expect("seed source tables");
}

fn run_smelt(project_dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(smelt_bin())
        .arg("run")
        .args(args)
        .arg("--project-dir")
        .arg(project_dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run`: {e}"))
}

fn silver_dates(db_path: &Path) -> Vec<String> {
    let conn = Connection::open(db_path).expect("open duckdb");
    let mut stmt = conn
        .prepare("SELECT CAST(d AS VARCHAR) FROM main.silver ORDER BY d")
        .expect("prepare");
    stmt.query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

/// Two `--source`/`--landed` deltas in one invocation drive different
/// (disjoint) regions of `silver`: bronze's delta reflects through its
/// zero-margin same-axis clamp to exactly its own day; aux's delta reflects
/// through the derived ±1-day window clamp to a 3-day span. Partitions
/// outside the union of these two propagated regions are never scheduled —
/// asserted directly against the materialized table (not just the printed
/// report).
#[test]
fn runs_exactly_the_propagated_regions() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_workspace(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_sources(&db_path);

    let output = run_smelt(
        &project_dir,
        &[
            "--since-upstream",
            "--source",
            "sources.bronze",
            "--landed",
            "2026-01-03..2026-01-04",
            "--source",
            "sources.aux",
            "--landed",
            "2026-01-07..2026-01-08",
        ],
    );
    assert!(
        output.status.success(),
        "since-upstream run must succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Dirty set (--since-upstream):"),
        "must print the dirty set before acting: {stdout}"
    );
    assert!(stdout.contains("silver <- bronze"), "{stdout}");
    assert!(stdout.contains("silver <- aux"), "{stdout}");

    // bronze's delta [Jan3,Jan4) => silver day Jan3 only.
    // aux's delta [Jan7,Jan8) reflects through the ±1d clamp => [Jan6,Jan9).
    let expected = vec![
        "2026-01-03".to_string(),
        "2026-01-06".to_string(),
        "2026-01-07".to_string(),
        "2026-01-08".to_string(),
    ];
    assert_eq!(
        silver_dates(&db_path),
        expected,
        "only the propagated regions may be materialized — nothing outside them"
    );
}

/// A source with no matching `--landed` interval contributes no dirt — no
/// implicit whole-table or recorded-state fallback. Only `bronze`'s delta is
/// declared here; `aux` (also read by `silver`) must not appear in the
/// dirty-set report at all, and only bronze's single day is materialized.
#[test]
fn source_without_landed_flag_propagates_nothing() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_workspace(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_sources(&db_path);

    let output = run_smelt(
        &project_dir,
        &[
            "--since-upstream",
            "--source",
            "sources.bronze",
            "--landed",
            "2026-01-05..2026-01-06",
        ],
    );
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("silver <- bronze"), "{stdout}");
    assert!(
        !stdout.contains("silver <- aux"),
        "aux declared no delta and must not appear in the dirty set: {stdout}"
    );
    assert_eq!(silver_dates(&db_path), vec!["2026-01-05".to_string()]);
}

/// Running `--since-upstream` over the propagated regions must leave those
/// regions equal to what a full refresh over complete history would have
/// computed for the same partitions (`maintenance_plan.md` §"The graph
/// layer": "must leave every model equal to a full refresh"). Compared via
/// row-level equality on the dirtied dates between a since-upstream run and
/// an independent full-refresh run of the same fixture.
#[test]
fn sufficiency_equals_full_refresh() {
    let tmp = TempDir::new().unwrap();

    // Partial run via --since-upstream, under its own subdirectory.
    let partial_parent = tmp.path().join("partial");
    std::fs::create_dir_all(&partial_parent).unwrap();
    let partial_dir = stage_workspace(&partial_parent);
    let partial_db = partial_dir.join("target/dev.duckdb");
    seed_sources(&partial_db);
    let out = run_smelt(
        &partial_dir,
        &[
            "--since-upstream",
            "--source",
            "sources.bronze",
            "--landed",
            "2026-01-03..2026-01-04",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Full refresh over the same fixture, in a wholly independent workspace.
    let full_parent = tmp.path().join("full");
    std::fs::create_dir_all(&full_parent).unwrap();
    let full_dir = stage_workspace(&full_parent);
    let full_db = full_dir.join("target/dev.duckdb");
    seed_sources(&full_db);
    let out = run_smelt(
        &full_dir,
        &[
            "--event-time-start",
            "2026-01-01",
            "--event-time-end",
            "2026-01-11",
        ],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Row-level equality restricted to the partial run's dirtied date.
    let partial_conn = Connection::open(&partial_db).expect("open partial db");
    let mut stmt = partial_conn
        .prepare(
            "SELECT id, CAST(d AS VARCHAR), aux_id FROM main.silver WHERE d = DATE '2026-01-03'",
        )
        .unwrap();
    let partial_row: (i32, String, i32) = stmt
        .query_row([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("partial row for Jan3");

    let full_conn = Connection::open(&full_db).expect("open full db");
    let mut stmt = full_conn
        .prepare(
            "SELECT id, CAST(d AS VARCHAR), aux_id FROM main.silver WHERE d = DATE '2026-01-03'",
        )
        .unwrap();
    let full_row: (i32, String, i32) = stmt
        .query_row([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("full row for Jan3");

    assert_eq!(
        partial_row, full_row,
        "the since-upstream run's dirtied region must match a full refresh over the same period"
    );
}

/// A self-referential model (a ref to its own address) refuses fail-loud —
/// `MaintenanceGraphUnsupportedNode` — before any interval math runs, never
/// silently treated as a day axis.
#[test]
fn self_referential_node_refuses_fail_loud() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("proj");
    write(
        &root,
        "smelt.yml",
        "name: selfref_ws\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\n",
    );
    write(
        &root,
        "models/rolling.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT id, d FROM smelt.rolling\n",
    );
    std::fs::create_dir_all(root.join("target")).unwrap();

    let output = run_smelt(
        &root,
        &[
            "--since-upstream",
            "--source",
            "sources.bronze",
            "--landed",
            "2026-01-01..2026-01-02",
        ],
    );
    assert!(
        !output.status.success(),
        "self-referential node must refuse"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("MaintenanceGraphUnsupportedNode"),
        "refusal must name the diagnostic: {stderr}"
    );
    assert!(stderr.contains("self-referential"), "{stderr}");
    assert!(
        !stderr.contains("panicked at"),
        "must be a named error, not a panic: {stderr}"
    );
}

/// `--landed` without a matching `--source` (count mismatch), or a
/// malformed interval, is a named CLI error, never a panic.
#[test]
fn malformed_landed_range_errors() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_workspace(tmp.path());
    seed_sources(&project_dir.join("target/dev.duckdb"));

    // Mismatched counts: two --source, one --landed.
    let output = run_smelt(
        &project_dir,
        &[
            "--since-upstream",
            "--source",
            "sources.bronze",
            "--source",
            "sources.aux",
            "--landed",
            "2026-01-01..2026-01-02",
        ],
    );
    assert!(!output.status.success(), "count mismatch must error");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--source and --landed"), "{stderr}");
    assert!(!stderr.contains("panicked at"), "{stderr}");

    // Malformed interval syntax.
    let output = run_smelt(
        &project_dir,
        &[
            "--since-upstream",
            "--source",
            "sources.bronze",
            "--landed",
            "not-a-range",
        ],
    );
    assert!(!output.status.success(), "malformed range must error");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("malformed --landed range"), "{stderr}");
    assert!(!stderr.contains("panicked at"), "{stderr}");
}
