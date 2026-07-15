#![cfg(feature = "duckdb")]
//! Phase MP16 (`docs/plans/20260707-maintenance-plan-impl.md`): backward
//! resolution — `smelt build <model> --period <start>..<end>
//! --include-upstreams` (`incremental_models.md` §CLI, §"Backward resolution —
//! what must exist"). Given a target model and a requested output period,
//! walk the ancestor sub-DAG backward through the SAME per-workspace `Edge`
//! graph `--since-upstream` assembles (`smelt_runtime::propagation::
//! build_forward_graph`), resolve the per-ancestor required slices and the
//! ancestor-first/target-last build order, print them, and execute exactly
//! that bounded build.
//!
//! Fixture mirrors `since_upstream.rs`: a clocked `bronze` source feeding
//! `silver` (grain: partition), silver feeding `gold` (an aggregate with a
//! 1-day trailing lookback) — a real two-hop chain so the backward
//! resolution composes through more than one edge, and `build_order` has
//! more than one entry to assert ancestor-first/target-last on.

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

fn smelt_yml(root: &Path, name: &str) {
    write(
        root,
        "smelt.yml",
        &format!(
            "name: {name}\nversion: 1\npaths:\n  - models\n\
             targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
             default_materialization: view\n"
        ),
    );
}

/// `bronze` (clocked, append-only) -> `silver` (grain: partition, same-axis
/// zero-margin read) -> `gold` (grain: partition, 1-day trailing lookback
/// over `silver`).
fn stage_chain_workspace(root: &Path) {
    smelt_yml(root, "include_upstreams_ws");
    write(
        root,
        "models/sources/bronze.yml",
        "description: bronze\ncolumns:\n- name: id\n  type: INTEGER\n- name: d\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    );
    write(
        root,
        "models/silver.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT id, d FROM smelt.sources.bronze\n",
    );
    write(
        root,
        "models/gold.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT s.id, s.d, COUNT(*) OVER (\n\
         \x20\x20PARTITION BY s.id ORDER BY s.d\n\
         \x20\x20RANGE BETWEEN INTERVAL '1 day' PRECEDING AND CURRENT ROW\n\
         ) AS trailing_count\nFROM smelt.silver s\n",
    );
    std::fs::create_dir_all(root.join("target")).unwrap();
}

/// Pre-populate `main.sources_bronze` with 10 days of data
/// (2026-01-01 .. 2026-01-10), one row per day.
fn seed_bronze(db_path: &Path) {
    let conn = Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        "CREATE SCHEMA IF NOT EXISTS main;\n\
         CREATE TABLE main.sources_bronze (id INTEGER, d DATE);\n\
         INSERT INTO main.sources_bronze \
           SELECT i, DATE '2026-01-01' + CAST(i - 1 AS INTEGER) FROM range(1, 11) t(i);\n",
    )
    .expect("seed bronze");
}

/// Pre-populate `main.sources_bronze` with ONLY the days in
/// `[start_day, end_day)` (an offset from 2026-01-01, end exclusive) — used
/// to prove a resolved required slice actually suffices: if it were
/// under-computed, the rows this omits would be missing and the downstream
/// build would produce wrong (or fail to produce any) output for the
/// requested period.
fn seed_bronze_range(db_path: &Path, start_day: i64, end_day: i64) {
    let conn = Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(&format!(
        "CREATE SCHEMA IF NOT EXISTS main;\n\
         CREATE TABLE main.sources_bronze (id INTEGER, d DATE);\n\
         INSERT INTO main.sources_bronze \
           SELECT i, DATE '2026-01-01' + CAST(i - 1 AS INTEGER) \
           FROM range({start_start}, {end_start}) t(i);\n",
        start_start = start_day + 1,
        end_start = end_day + 1,
    ))
    .expect("seed bronze range");
}

fn run_smelt(project_dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(smelt_bin())
        .arg("build")
        .args(args)
        .arg("--project-dir")
        .arg(project_dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build`: {e}"))
}

fn table_dates(db_path: &Path, table: &str) -> Vec<String> {
    let conn = Connection::open(db_path).expect("open duckdb");
    let mut stmt = conn
        .prepare(&format!(
            "SELECT CAST(d AS VARCHAR) FROM main.{table} ORDER BY d"
        ))
        .expect("prepare");
    stmt.query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

/// Staging ONLY the resolved bronze slice (bronze's widened requirement from
/// gold's 1-day trailing lookback through silver's zero-margin read — 2 days,
/// 2026-01-04 and 2026-01-05, for gold's requested 2026-01-05..2026-01-06
/// output period) and building bottom-up (silver, then gold) reproduces a
/// build over complete history for the same target period — the driving use
/// case for the bounded test/validation build (`incremental_models.md`
/// §"Backward resolution — what must exist"). Seeding bronze with ONLY the
/// resolved slice (not the full 10-day history) is what actually exercises
/// "the resolved slice suffices": if `resolve_build_plan` under-computed the
/// required bronze interval, the row(s) it omitted would be missing here and
/// gold's `trailing_count` would come out wrong (or the build would fail
/// outright) for 2026-01-05, diverging from the full-history oracle below.
#[test]
fn resolved_slices_suffice() {
    let tmp = TempDir::new().unwrap();

    // Partial build via --include-upstreams, under its own subdirectory.
    // Bronze is seeded with ONLY the interval `resolve_build_plan` is
    // expected to report as required — offsets 3..5 from 2026-01-01, i.e.
    // 2026-01-04 and 2026-01-05 — asserted below via the printed report, so
    // this test would fail (rather than pass vacuously) if the resolution
    // were too narrow or too wide.
    let partial_parent = tmp.path().join("partial");
    std::fs::create_dir_all(&partial_parent).unwrap();
    let partial_dir = partial_parent.join("proj");
    stage_chain_workspace(&partial_dir);
    let partial_db = partial_dir.join("target/dev.duckdb");
    seed_bronze_range(&partial_db, 3, 5);

    let output = run_smelt(
        &partial_dir,
        &[
            "gold",
            "--period",
            "2026-01-05..2026-01-06",
            "--include-upstreams",
        ],
    );
    assert!(
        output.status.success(),
        "--include-upstreams build must succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("--include-upstreams"),
        "must print the resolved-slices report before acting: {stdout}"
    );
    assert!(stdout.contains("silver"), "{stdout}");
    assert!(stdout.contains("gold"), "{stdout}");

    // Pin the exact resolved bronze interval — this is the assertion that
    // makes the seeded range above meaningful, not an arbitrary guess: if
    // `resolve_build_plan` ever resolved a different (too-narrow or
    // too-wide) bronze interval, this line changes and the test fails here
    // rather than silently seeding the "old" range and passing anyway.
    assert!(
        stdout.contains("STAGE bronze: [2026-01-04, 2026-01-06)"),
        "resolved bronze interval must be exactly [2026-01-04, 2026-01-06) (gold's 1-day \
         trailing lookback widened through silver's zero-margin read): {stdout}"
    );

    // Ancestor-first, target-last: "silver" must appear before "gold" in
    // the printed build order.
    let build_order_line = stdout
        .lines()
        .find(|l| l.starts_with("Build order:"))
        .unwrap_or_else(|| panic!("no 'Build order:' line: {stdout}"));
    let silver_pos = build_order_line.find("silver").expect("silver in order");
    let gold_pos = build_order_line.find("gold").expect("gold in order");
    assert!(
        silver_pos < gold_pos,
        "build order must be ancestor-first, target-last: {build_order_line}"
    );

    // Full refresh over the same fixture, in a wholly independent workspace.
    let full_parent = tmp.path().join("full");
    std::fs::create_dir_all(&full_parent).unwrap();
    let full_dir = full_parent.join("proj");
    stage_chain_workspace(&full_dir);
    let full_db = full_dir.join("target/dev.duckdb");
    seed_bronze(&full_db);
    let out = Command::new(smelt_bin())
        .arg("run")
        .args([
            "--event-time-start",
            "2026-01-01",
            "--event-time-end",
            "2026-01-11",
        ])
        .arg("--project-dir")
        .arg(&full_dir)
        .output()
        .expect("full refresh run");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Row-level equality restricted to gold's requested period.
    let partial_conn = Connection::open(&partial_db).expect("open partial db");
    let mut stmt = partial_conn
        .prepare(
            "SELECT id, CAST(d AS VARCHAR), trailing_count FROM main.gold \
             WHERE d = DATE '2026-01-05' ORDER BY id",
        )
        .unwrap();
    let partial_rows: Vec<(i32, String, i32)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();

    let full_conn = Connection::open(&full_db).expect("open full db");
    let mut stmt = full_conn
        .prepare(
            "SELECT id, CAST(d AS VARCHAR), trailing_count FROM main.gold \
             WHERE d = DATE '2026-01-05' ORDER BY id",
        )
        .unwrap();
    let full_rows: Vec<(i32, String, i32)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();

    assert!(!full_rows.is_empty(), "sanity: full refresh must have rows");
    assert_eq!(
        partial_rows, full_rows,
        "the --include-upstreams build's resolved period must match a full refresh over the \
         same fixture"
    );

    // The resolved silver slice must also be populated (an intermediate
    // ancestor, not just the target).
    assert!(
        !table_dates(&partial_db, "silver").is_empty(),
        "silver (an ancestor model) must have been built, not just gold"
    );
}

/// An ancestor whose partition grain can't be sliced (an unclocked
/// dim/lookup source, no `timeseries:` declared) must be staged/built whole
/// — the required slice is the whole table, never a bounded interval
/// (`incremental_models.md` §"Backward resolution — what must exist": "The
/// required slice of an unclocked source is the whole table").
#[test]
fn unclocked_ancestor_requires_whole_table() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("proj");
    smelt_yml(&root, "unclocked_ancestor_ws");
    write(
        &root,
        "models/sources/bronze.yml",
        "description: bronze\ncolumns:\n- name: id\n  type: INTEGER\n- name: d\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    );
    // `dim` has no `timeseries:` block — an unclocked lookup source.
    write(
        &root,
        "models/sources/dim.yml",
        "description: dim\ncolumns:\n- name: id\n  type: INTEGER\n\
         - name: label\n  type: VARCHAR\n\
         mutation_profile:\n  kind: mutable_snapshot\n",
    );
    write(
        &root,
        "models/silver.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n\
         maintenance:\n  scan_bounds:\n    per_source:\n      dim:\n        allow_full_scan: true\n\
         ---\n\
         SELECT b.id, b.d, dm.label\nFROM smelt.sources.bronze b\n\
         LEFT JOIN smelt.sources.dim dm ON dm.id = b.id\n",
    );
    std::fs::create_dir_all(root.join("target")).unwrap();
    let db_path = root.join("target/dev.duckdb");
    seed_bronze(&db_path);
    {
        let conn = Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            "CREATE TABLE main.sources_dim (id INTEGER, label VARCHAR);\n\
             INSERT INTO main.sources_dim VALUES (1, 'one'), (2, 'two');\n",
        )
        .expect("seed dim");
    }

    let output = run_smelt(
        &root,
        &[
            "silver",
            "--period",
            "2026-01-05..2026-01-06",
            "--include-upstreams",
        ],
    );
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("dim") && stdout.contains("whole table"),
        "the unclocked ancestor 'dim' must be reported as requiring the whole table: {stdout}"
    );
}

/// Regression: a target with NO inbound edge in the shared propagation graph
/// (here, `flat` is `refresh: full` — `build_forward_graph` never derives a
/// maintenance plan or edges for a non-incremental model) must still
/// actually get built via `--include-upstreams`, not silently skipped.
/// Before the fix, `required_inputs`'s `build_order` filter excluded any
/// node without an inbound edge — including the target itself — so
/// `resolve_build_plan` returned an empty `build_order`, and
/// `build_include_upstreams` read that as "nothing to build," printed a
/// warning, and exited 0 without ever running the model the user asked for.
#[test]
fn full_refresh_target_with_no_inbound_edge_still_builds() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("proj");
    smelt_yml(&root, "no_inbound_edge_ws");
    write(
        &root,
        "models/sources/bronze.yml",
        "description: bronze\ncolumns:\n- name: id\n  type: INTEGER\n- name: d\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    );
    // `flat` is `refresh: full` (the default) — `build_forward_graph` never
    // derives a maintenance-plan cell or any `Edge` for it, so it has no
    // inbound edge in the shared graph.
    write(
        &root,
        "models/flat.sql",
        "---\nmaterialization: table\n---\nSELECT id, d FROM smelt.sources.bronze\n",
    );
    std::fs::create_dir_all(root.join("target")).unwrap();
    let db_path = root.join("target/dev.duckdb");
    seed_bronze(&db_path);

    let output = run_smelt(
        &root,
        &[
            "flat",
            "--period",
            "2026-01-05..2026-01-06",
            "--include-upstreams",
        ],
    );
    assert!(
        output.status.success(),
        "--include-upstreams build must succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("resolved nothing to build"),
        "a no-inbound-edge target must not be silently skipped: {stdout}"
    );
    assert!(
        stdout.contains("BUILD flat"),
        "the target itself must be reported as BUILD, not just STAGE: {stdout}"
    );

    let conn = Connection::open(&db_path).expect("open duckdb");
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM main.flat", [], |r| r.get(0))
        .expect("flat must have been built with rows, not silently skipped");
    assert_eq!(
        count, 10,
        "flat must contain all 10 seeded bronze rows (a full-refresh build reads the whole \
         source, since `flat` has no incremental config to bound it)"
    );
}
