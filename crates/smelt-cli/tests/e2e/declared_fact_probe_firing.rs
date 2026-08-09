//! End-to-end coverage that a declared model-scoped fact (`functional_dependencies:`,
//! `bounded_domain:`) violated in real data fails the run *before* any write —
//! the live dispatch this outcome's phase 5 wires at the pre-write sites in
//! `crates/smelt-runtime/src/execute.rs`
//! (`docs/outcomes/20260809-probe-backed-facts/phases/05-plan.md`,
//! `docs/specs/model_properties.md` §"Probe obligation"). `model_probes.rs`
//! proves the builder/dispatcher pure functions directly; this file proves
//! the real pipeline — compiled binary, real DuckDB, real `smelt.yml` —
//! actually calls them.

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

fn combined_output(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    )
}

// ── Test 9: full-refresh `table` model, violated `functional_dependencies:` ──

fn stage_fd_workspace(tmp: &TempDir) -> PathBuf {
    let root = tmp.path().join("fd_probe");
    write_file(
        &root.join("smelt.yml"),
        r#"name: fd_probe
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main
default_materialization: table
"#,
    );
    write_file(
        &root.join("models/sources/raw/subs.yml"),
        r#"description: Raw subscription rows; pre-loaded by the integration test
name: raw.subs
columns:
  - name: customer_id
    type: INTEGER
  - name: region
    type: VARCHAR
"#,
    );
    write_file(
        &root.join("models/subscriptions.sql"),
        r#"---
materialization: table
functional_dependencies:
  - key: [customer_id]
    determines: region
---
SELECT customer_id, region FROM smelt.sources.raw.subs
"#,
    );
    root
}

fn seed_subs(db_path: &Path, violating: bool) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch("CREATE SCHEMA IF NOT EXISTS raw;")
        .unwrap();
    if violating {
        conn.execute_batch(
            "CREATE TABLE raw.subs (customer_id INTEGER, region VARCHAR); \
             INSERT INTO raw.subs VALUES (1, 'us'), (1, 'eu'), (2, 'us');",
        )
        .unwrap();
    } else {
        conn.execute_batch(
            "CREATE TABLE raw.subs (customer_id INTEGER, region VARCHAR); \
             INSERT INTO raw.subs VALUES (1, 'us'), (1, 'us'), (2, 'eu');",
        )
        .unwrap();
    }
}

fn run_build(project_dir: &Path, db_path: &Path) -> std::process::Output {
    Command::new(smelt_bin())
        .args([
            "build",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--database",
            db_path.to_str().unwrap(),
        ])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"))
}

fn table_exists(db_path: &Path, schema_qualified: &str) -> bool {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.query_row(
        &format!("SELECT COUNT(*) FROM {schema_qualified}"),
        [],
        |row| row.get::<_, i64>(0),
    )
    .is_ok()
}

#[test]
fn violating_fd_fails_the_run_before_any_write() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_fd_workspace(&tmp);
    let db_path = ws.join("target/dev.duckdb");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    seed_subs(&db_path, true);

    let output = run_build(&ws, &db_path);
    let combined = combined_output(&output);

    assert!(
        !output.status.success(),
        "a violated functional dependency must fail the build; got:\n{combined}"
    );
    assert!(
        combined.contains("DeclaredFunctionalDependencyViolated"),
        "expected the named diagnostic; got:\n{combined}"
    );
    assert!(
        !table_exists(&db_path, "main.subscriptions"),
        "the probe must fire before any write — main.subscriptions must not exist"
    );
}

// ── Tests 10/11: incremental batch, violated `bounded_domain:` ──

fn stage_bounded_domain_workspace(tmp: &TempDir, name: &str, probes_off: bool) -> PathBuf {
    let root = tmp.path().join(name);
    let probes_block = if probes_off {
        "probes:\n  cadence: off\n"
    } else {
        ""
    };
    write_file(
        &root.join("smelt.yml"),
        &format!(
            r#"name: {name}
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main
default_materialization: table
{probes_block}"#
        ),
    );
    write_file(
        &root.join("models/sources/raw/events.yml"),
        r#"description: Raw event rows; pre-loaded by the integration test
name: raw.events
columns:
  - name: event_ts
    type: TIMESTAMP
  - name: country_code
    type: VARCHAR
"#,
    );
    write_file(
        &root.join("models/daily_countries.sql"),
        r#"---
materialization: table
refresh: incremental
grain: partition
bounded_domain:
  column: country_code
  max_cardinality: 2
timeseries:
  event_time_column: event_ts
  partition_column: event_date
  granularity: day
---
SELECT CAST(event_ts AS DATE) AS event_date, country_code
FROM smelt.sources.raw.events
"#,
    );
    root
}

fn seed_events(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch("CREATE SCHEMA IF NOT EXISTS raw;")
        .unwrap();
    conn.execute_batch(
        "CREATE TABLE raw.events (event_ts TIMESTAMP, country_code VARCHAR); \
         INSERT INTO raw.events VALUES \
           (TIMESTAMP '2026-01-01 00:00:00', 'US'), \
           (TIMESTAMP '2026-01-01 01:00:00', 'CA'); \
         INSERT INTO raw.events VALUES \
           (TIMESTAMP '2026-01-02 00:00:00', 'US'), \
           (TIMESTAMP '2026-01-02 01:00:00', 'CA'), \
           (TIMESTAMP '2026-01-02 02:00:00', 'MX'), \
           (TIMESTAMP '2026-01-02 03:00:00', 'FR');",
    )
    .unwrap();
}

fn run_window(project_dir: &Path, db_path: &Path, start: &str, end: &str) -> std::process::Output {
    Command::new(smelt_bin())
        .args([
            "run",
            "--project-dir",
            project_dir.to_str().unwrap(),
            "--database",
            db_path.to_str().unwrap(),
            "--event-time-start",
            start,
            "--event-time-end",
            end,
        ])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run`: {e}"))
}

fn read_rows(db_path: &Path) -> Vec<(String, String)> {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    let mut stmt = conn
        .prepare(
            "SELECT CAST(event_date AS VARCHAR), country_code FROM main.daily_countries \
             ORDER BY event_date, country_code",
        )
        .unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

#[test]
fn violating_bounded_domain_fails_an_incremental_batch_before_its_write() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_bounded_domain_workspace(&tmp, "bounded_domain_probe", false);
    let db_path = ws.join("target/dev.duckdb");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    seed_events(&db_path);

    // Day 1 conforms (2 distinct countries, cap is 2) — builds the target.
    let ok = run_window(&ws, &db_path, "2026-01-01", "2026-01-02");
    assert!(
        ok.status.success(),
        "conforming day must build; got:\n{}",
        combined_output(&ok)
    );
    let before = read_rows(&db_path);
    assert_eq!(before.len(), 2, "day 1 must have written exactly 2 rows");

    // Day 2 violates (4 distinct countries > cap of 2) — must fail before write.
    let violating = run_window(&ws, &db_path, "2026-01-02", "2026-01-03");
    let combined = combined_output(&violating);
    assert!(
        !violating.status.success(),
        "a violated bounded domain must fail the batch; got:\n{combined}"
    );
    assert!(
        combined.contains("DeclaredBoundedDomainExceeded"),
        "expected the named diagnostic; got:\n{combined}"
    );

    let after = read_rows(&db_path);
    assert_eq!(
        before, after,
        "the target's pre-run contents must be unchanged after a refused batch"
    );
}

#[test]
fn probes_off_lets_the_violating_run_write() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_bounded_domain_workspace(&tmp, "bounded_domain_probe_off", true);
    let db_path = ws.join("target/dev.duckdb");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    seed_events(&db_path);

    let violating = run_window(&ws, &db_path, "2026-01-02", "2026-01-03");
    assert!(
        violating.status.success(),
        "probes: {{cadence: off}} must let the violating run write; got:\n{}",
        combined_output(&violating)
    );

    let rows = read_rows(&db_path);
    assert_eq!(
        rows.len(),
        4,
        "the violating batch's 4 distinct-country rows must have been written"
    );
}

// ── Tests 12/13: full-refresh model, source append-only posture probe ──

fn stage_append_only_workspace(tmp: &TempDir, name: &str, probes_off: bool) -> PathBuf {
    let root = tmp.path().join(name);
    let probes_block = if probes_off {
        "probes:\n  cadence: off\n"
    } else {
        ""
    };
    write_file(
        &root.join("smelt.yml"),
        &format!(
            r#"name: {name}
version: 1
paths:
  - models
targets:
  dev:
    type: duckdb
    database: target/dev.duckdb
    schema: main
default_materialization: table
{probes_block}"#
        ),
    );
    write_file(
        &root.join("models/sources/raw/clicks.yml"),
        r#"description: Raw append-only clickstream rows; pre-loaded by the integration test
name: raw.clicks
mutation_profile:
  kind: append_only
timeseries:
  event_time_column: click_ts
  partition_column: click_date
  granularity: day
columns:
  - name: click_ts
    type: TIMESTAMP
  - name: click_date
    type: DATE
  - name: payload
    type: VARCHAR
"#,
    );
    write_file(
        &root.join("models/clicks_summary.sql"),
        r#"---
materialization: table
---
SELECT click_date, payload FROM smelt.sources.raw.clicks
"#,
    );
    root
}

/// Two partitions so one (`2026-01-01`) is closed by the time
/// `2026-01-02` is the recorded maximum — the frontier gate
/// (`docs/specs/model_properties.md` §"Probe obligation") only
/// fingerprint-checks the closed one.
fn seed_clicks(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch("CREATE SCHEMA IF NOT EXISTS raw;")
        .unwrap();
    conn.execute_batch(
        "CREATE TABLE raw.clicks (click_ts TIMESTAMP, click_date DATE, payload VARCHAR); \
         INSERT INTO raw.clicks VALUES \
           (TIMESTAMP '2026-01-01 00:00:00', DATE '2026-01-01', 'a'), \
           (TIMESTAMP '2026-01-02 00:00:00', DATE '2026-01-02', 'b');",
    )
    .unwrap();
}

/// Mutates the CLOSED partition's content in place — same row count, changed
/// payload — the exact posture violation the append-only probe exists to
/// catch.
fn mutate_closed_partition(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        "UPDATE raw.clicks SET payload = 'mutated' WHERE click_date = DATE '2026-01-01';",
    )
    .unwrap();
}

fn read_clicks_summary(db_path: &Path) -> Vec<(String, String)> {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    let mut stmt = conn
        .prepare(
            "SELECT CAST(click_date AS VARCHAR), payload FROM main.clicks_summary \
             ORDER BY click_date, payload",
        )
        .unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

#[test]
fn append_only_source_mutated_between_runs_fails_the_second_run() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_append_only_workspace(&tmp, "append_only_probe", false);
    let db_path = ws.join("target/dev.duckdb");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    seed_clicks(&db_path);

    // First run: no recorded baseline yet, so no probe fires — the run
    // succeeds and records the baseline for next time.
    let first = run_build(&ws, &db_path);
    assert!(
        first.status.success(),
        "the first run must succeed and record the baseline; got:\n{}",
        combined_output(&first)
    );
    let after_first = read_clicks_summary(&db_path);
    assert_eq!(after_first.len(), 2, "the first run must write both rows");

    // Mutate the closed partition's content in place between runs.
    mutate_closed_partition(&db_path);

    // Second run: the recorded baseline's fingerprint for the closed
    // partition no longer matches — the probe must fire before any write.
    let second = run_build(&ws, &db_path);
    let combined = combined_output(&second);
    assert!(
        !second.status.success(),
        "a mutated closed partition must fail the second run; got:\n{combined}"
    );
    assert!(
        combined.contains("SourceMutationProfileViolated"),
        "expected the named diagnostic; got:\n{combined}"
    );

    // The model table's own content is unchanged from the first run's
    // write — the probe fired before the second run's write reached it.
    // (The source table itself was mutated directly by the test, so
    // re-reading it would show the mutation; what must NOT have changed is
    // the model's own materialized output.)
    let after_second = read_clicks_summary(&db_path);
    assert_eq!(
        after_first, after_second,
        "the model table must be unchanged after a refused second run"
    );
}

#[test]
fn probes_cadence_off_lets_the_mutating_run_write() {
    let tmp = TempDir::new().unwrap();
    let ws = stage_append_only_workspace(&tmp, "append_only_probe_off", true);
    let db_path = ws.join("target/dev.duckdb");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    seed_clicks(&db_path);

    let first = run_build(&ws, &db_path);
    assert!(
        first.status.success(),
        "the first run must succeed; got:\n{}",
        combined_output(&first)
    );

    mutate_closed_partition(&db_path);

    let second = run_build(&ws, &db_path);
    assert!(
        second.status.success(),
        "probes: {{cadence: off}} must let the mutating run write; got:\n{}",
        combined_output(&second)
    );

    let rows = read_clicks_summary(&db_path);
    assert!(
        rows.iter().any(|(_, payload)| payload == "mutated"),
        "the mutated payload must have been written through: {rows:?}"
    );
}
