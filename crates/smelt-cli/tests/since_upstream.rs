#![cfg(feature = "duckdb")]
//! MP15 (`docs/plans/20260707-maintenance-plan-impl.md`): `smelt run
//! --since-upstream` — forward propagation from per-source deltas
//! (`incremental_models.md` §CLI, §"The graph layer"): either declared
//! directly (`--source <address> --landed <start>..<end>`, repeatable) or,
//! for a source with no paired `--landed`, resolved from its persisted
//! `smelt-state` watermark (`run_state.md` §"Per-source watermark").
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
    // The model's own output/partition column is aliased to `event_date`
    // (a straight passthrough of bronze's `d`, not a derived value) rather
    // than left as bare `d` — so it reads distinctly from the join's anchor
    // expression `b.d` in `a.d2 BETWEEN b.d - INTERVAL ... AND b.d + INTERVAL
    // ...`. A textual skew-bound scan
    // (`smelt_logical::analysis::walk::model_partition_skew`) matches a Form
    // B relation whose anchor identifier equals the declared
    // `partition_column`; with `partition_column: d` and anchor `b.d`, that
    // match fires even though `d` here is a straight passthrough with no
    // actual derivation/skew (the same shape `examples/timeseries`'s real
    // `daily_events_status.sql` avoids by truncating its event-time column
    // into a differently-named `event_date`, `models/daily_events_status.sql`)
    // — aliasing to `event_date` here keeps this fixture isolated to the
    // propagation/reflection mechanism it actually exercises.
    write(
        &root,
        "models/silver.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: event_date\n  event_time_column: event_date\n  granularity: day\n---\n\
         SELECT b.id, b.d AS event_date, a.id AS aux_id\nFROM smelt.sources.bronze b\n\
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
        .prepare("SELECT CAST(event_date AS VARCHAR) FROM main.silver ORDER BY event_date")
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
/// computed for the same partitions (`incremental_models.md` §"The graph
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
            "SELECT id, CAST(event_date AS VARCHAR), aux_id FROM main.silver WHERE event_date = DATE '2026-01-03'",
        )
        .unwrap();
    let partial_row: (i32, String, i32) = stmt
        .query_row([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .expect("partial row for Jan3");

    let full_conn = Connection::open(&full_db).expect("open full db");
    let mut stmt = full_conn
        .prepare(
            "SELECT id, CAST(event_date AS VARCHAR), aux_id FROM main.silver WHERE event_date = DATE '2026-01-03'",
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
    // A resolvable declared source so `--source sources.bronze` passes the
    // `resolve_ref_path` precondition and the self-referential graph refusal
    // (not an unknown-address error) is what surfaces.
    write(
        &root,
        "models/sources/bronze.yml",
        "description: bronze\ncolumns:\n- name: id\n  type: INTEGER\n- name: d\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
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

/// Stage a model->model chain under `parent/proj`: `silver` (maintained,
/// partition-grain, clock `d`) reads `sources.bronze`; `gold` (maintained,
/// partition-grain, clock `d`) reads `smelt.silver` as a passthrough. The
/// `silver -> gold` edge is a maintained-model edge — the same one
/// `smelt explain gold` reports. Returns the project dir.
fn stage_model_chain(parent: &Path) -> PathBuf {
    let root = parent.join("proj");
    write(
        &root,
        "smelt.yml",
        "name: model_chain_ws\nversion: 1\npaths:\n  - models\n\
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
        "models/silver.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT id, d FROM smelt.sources.bronze\n",
    );
    write(
        &root,
        "models/gold.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT id, d FROM smelt.silver\n",
    );
    std::fs::create_dir_all(root.join("target")).unwrap();
    root
}

/// Pre-populate `main.silver` (the maintained model's own output) with 10
/// days of data — the delta origin's completed run is already materialized,
/// so `--since-upstream --source silver` reads it directly.
fn seed_silver(db_path: &Path) {
    let conn = Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        "CREATE SCHEMA IF NOT EXISTS main;\n\
         CREATE TABLE main.silver (id INTEGER, d DATE);\n\
         INSERT INTO main.silver \
           SELECT i, DATE '2026-01-01' + CAST(i - 1 AS INTEGER) FROM range(1, 11) t(i);\n",
    )
    .expect("seed silver table");
}

fn gold_dates(db_path: &Path) -> Vec<String> {
    let conn = Connection::open(db_path).expect("open duckdb");
    let mut stmt = conn
        .prepare("SELECT CAST(d AS VARCHAR) FROM main.gold ORDER BY d")
        .expect("prepare");
    stmt.query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

/// `--source <model-address>` accepts an upstream **maintained model** as the
/// delta origin (`incremental_models.md` §"Upstream model edges"): a landed
/// window declared on `silver` dirties only its downstream `gold`, the origin
/// model is never re-run, and `gold` materializes exactly the propagated
/// region.
#[test]
fn model_address_landed_delta_propagates() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_model_chain(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_silver(&db_path);

    let output = run_smelt(
        &project_dir,
        &[
            "--since-upstream",
            "--source",
            "silver",
            "--landed",
            "2026-01-03..2026-01-04",
        ],
    );
    assert!(
        output.status.success(),
        "model-origin since-upstream run must succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("gold <- silver"),
        "the dirty set must show the model edge: {stdout}"
    );
    assert!(
        !stdout.contains("RUN silver"),
        "the origin model must not be re-run: {stdout}"
    );

    // silver's delta [Jan3,Jan4) reflects zero-margin to gold day Jan3.
    assert_eq!(
        gold_dates(&db_path),
        vec!["2026-01-03".to_string()],
        "only the propagated region of gold may be materialized"
    );
}

/// A `--source` address that is neither a declared source nor a maintained
/// model in this project is a named CLI error (non-zero exit), never a
/// silent no-op or a panic — resolution goes through the canonical
/// `resolve_ref_path` resolver (`cli.md` §"Argument resolution").
#[test]
fn model_address_unknown_is_error() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_model_chain(tmp.path());
    seed_silver(&project_dir.join("target/dev.duckdb"));

    let output = run_smelt(
        &project_dir,
        &[
            "--since-upstream",
            "--source",
            "ghost.nonexistent",
            "--landed",
            "2026-01-03..2026-01-04",
        ],
    );
    assert!(
        !output.status.success(),
        "an unknown --source address must exit non-zero"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ghost.nonexistent"),
        "the error must name the unresolved address: {stderr}"
    );
    assert!(!stderr.contains("panicked at"), "{stderr}");
}

/// Stage a workspace whose middle model is a **locality-admitted composed
/// node** (`grain: key` + `timeseries:`, route 1 key-embedded — the
/// partition column `d` is itself part of the `GROUP BY` key, the same
/// admission shape `examples/timeseries/models/user_daily_spend.sql` uses):
/// `bronze -> composed [grain: key, timeseries: d] -> gold [grain:
/// partition]`. Returns the project dir.
fn stage_composed_origin_workspace(parent: &Path) -> PathBuf {
    let root = parent.join("proj");
    write(
        &root,
        "smelt.yml",
        "name: composed_origin_ws\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\n",
    );
    write(
        &root,
        "models/sources/bronze.yml",
        "description: bronze\ncolumns:\n- name: event_id\n  type: INTEGER\n\
         - name: d\n  type: DATE\n- name: val\n  type: INTEGER\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    );
    write(
        &root,
        "models/composed.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT event_id, d, MIN(val) AS val\nFROM smelt.sources.bronze\n\
         GROUP BY event_id, d\n",
    );
    write(
        &root,
        "models/gold.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT event_id, d, val FROM smelt.composed\n",
    );
    std::fs::create_dir_all(root.join("target")).unwrap();
    root
}

/// Pre-populate `main.composed` (the composed model's own output) with 10
/// days of data — the delta origin's completed run is already materialized,
/// so `--since-upstream --source composed` reads it directly.
fn seed_composed(db_path: &Path) {
    let conn = Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        "CREATE SCHEMA IF NOT EXISTS main;\n\
         CREATE TABLE main.composed (event_id INTEGER, d DATE, val INTEGER);\n\
         INSERT INTO main.composed \
           SELECT i, DATE '2026-01-01' + CAST(i - 1 AS INTEGER), i \
           FROM range(1, 11) t(i);\n",
    )
    .expect("seed composed table");
}

fn gold_dates_composed(db_path: &Path) -> Vec<String> {
    let conn = Connection::open(db_path).expect("open duckdb");
    let mut stmt = conn
        .prepare("SELECT CAST(d AS VARCHAR) FROM main.gold ORDER BY d")
        .expect("prepare");
    stmt.query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

/// Phase B3 (`docs/plans/20260715-composed-axes-conditional-maintenance.md`):
/// `--source <address>` accepts a **locality-admitted composed model**
/// (`grain: key` + `timeseries:`) as the delta origin, not just a bare
/// `grain: partition`/`grain: key_per_partition` model
/// (`model_address_landed_delta_propagates` already covers the latter). A
/// landed window declared directly on `composed`'s own declared output axis
/// dirties only its downstream `gold`; the composed origin itself is never
/// re-run.
#[test]
fn composed_model_address_landed_delta_propagates() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_composed_origin_workspace(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_composed(&db_path);

    let output = run_smelt(
        &project_dir,
        &[
            "--since-upstream",
            "--source",
            "composed",
            "--landed",
            "2026-01-03..2026-01-04",
        ],
    );
    assert!(
        output.status.success(),
        "composed-origin since-upstream run must succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("gold <- composed"),
        "the dirty set must show the composed model edge: {stdout}"
    );
    assert!(
        !stdout.contains("RUN composed"),
        "the composed origin must not be re-run: {stdout}"
    );

    // composed's delta [Jan3,Jan4) reflects zero-margin to gold day Jan3.
    assert_eq!(
        gold_dates_composed(&db_path),
        vec!["2026-01-03".to_string()],
        "only the propagated region of gold may be materialized"
    );
}

/// Phase 6 (`docs/outcomes/20260816-scheduler-delta-signatures/outcome.md`):
/// `--since-upstream` now reads the recorded `_smelt_observed_delta` table
/// LIVE off the backend, not just the pure planner tests
/// (`crates/smelt-runtime/tests/since_upstream_propagation.rs`). A
/// present-and-empty row recorded for `composed`'s exact
/// `[2026-01-03, 2026-01-04)` window (the same window
/// `composed_model_address_landed_delta_propagates` drives a real, non-empty
/// propagation from) makes the CLI propagate nothing and run zero models —
/// the live "delta empty" leg, end to end through the real `smelt` binary.
/// `composed_model_address_landed_delta_propagates` itself is unchanged by
/// this phase (no recorded row for its window — the live read must still
/// fall back to the declared window, widen-never-narrow).
#[test]
fn since_upstream_consumes_recorded_empty_delta() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_composed_origin_workspace(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_composed(&db_path);

    // Record a present-and-empty observed delta for `composed`'s exact
    // window via the real emitter — the write-family recording gap
    // (nothing yet records through a real conditional write for this
    // fixture's technique) stays in the narrowed divergence bullet; this is
    // the same "record with the real generate_observed_delta_upsert_sql
    // against the target DuckDB file" fallback the phase plan calls for.
    {
        let conn = Connection::open(&db_path).expect("open duckdb");
        let ddl = smelt_state::ddl_duckdb::generate_observed_delta_table_ddl("main");
        conn.execute_batch(&ddl)
            .expect("create observed-delta table");
        let upsert = smelt_state::ddl_duckdb::generate_observed_delta_upsert_sql(
            "main",
            "composed",
            "2026-01-03",
            "2026-01-04",
            "SELECT NULL::VARCHAR AS delta_key, NULL::VARCHAR AS delta_partition WHERE FALSE",
        );
        conn.execute_batch(&upsert)
            .expect("record the present-and-empty delta");
    }

    let output = run_smelt(
        &project_dir,
        &[
            "--since-upstream",
            "--source",
            "composed",
            "--landed",
            "2026-01-03..2026-01-04",
        ],
    );
    assert!(
        output.status.success(),
        "a present-and-empty recorded delta must not be a refusal: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("gold <- composed"),
        "a present-and-empty recorded delta must show no dirt on the composed edge: {stdout}"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("propagated nothing"),
        "a present-and-empty recorded delta must propagate nothing: stdout={stdout} \
         stderr={stderr}"
    );
    // `gold` is never materialized at all — the live-read empty delta
    // schedules zero regions, so `run_since_upstream` exits before any
    // model runs (unlike `composed_model_address_landed_delta_propagates`,
    // which asserts `gold`'s dates once it HAS been created). Nothing
    // further to assert here — the stdout/stderr checks above already pin
    // the "propagated nothing" behavior.
}

/// A **bare** keyed model (`grain: key`, no `timeseries:` — never locality-
/// admitted) named as `--source` still refuses fail-loud, even though the
/// address itself resolves (`RefKind::Model`) and so passes the CLI's
/// resolution precondition: the graph-layer refusal (S12, `"without an
/// admitted time axis"`) is what actually surfaces, the same message
/// `resolve_ref_path`-adjacent `bare_keyed_upstream_still_refuses`
/// (`crates/smelt-runtime/tests/since_upstream_propagation.rs`) pins at the
/// assembly level — this test is the CLI's own end-to-end leg of the same
/// refusal.
#[test]
fn bare_keyed_source_still_refuses() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("proj");
    write(
        &root,
        "smelt.yml",
        "name: bare_keyed_origin_ws\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\n",
    );
    write(
        &root,
        "models/sources/payments.yml",
        "description: payments\ncolumns:\n- name: user_id\n  type: INTEGER\n\
         - name: amount\n  type: DECIMAL(10,2)\n\
         mutation_profile:\n  kind: append_only\n",
    );
    write(
        &root,
        "models/bare_keyed.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT user_id, SUM(amount) AS total\nFROM smelt.sources.payments\nGROUP BY user_id\n",
    );
    write(
        &root,
        "models/downstream.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: partition\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n---\n\
         SELECT user_id, total, CAST('2026-01-01' AS DATE) AS d FROM smelt.bare_keyed\n",
    );
    std::fs::create_dir_all(root.join("target")).unwrap();

    let output = run_smelt(
        &root,
        &[
            "--since-upstream",
            "--source",
            "bare_keyed",
            "--landed",
            "2026-01-03..2026-01-04",
        ],
    );
    assert!(
        !output.status.success(),
        "a bare keyed --source origin must refuse, not silently no-op"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("without an admitted time axis"),
        "must surface the graph-layer keyed refusal: {stderr}"
    );
    assert!(!stderr.contains("panicked at"), "{stderr}");
}

/// Stage a clockless `keyed upsert` model (`agg`) feeding a `grain:
/// partition` downstream (`downstream`) — the same shape
/// `crates/smelt-runtime/tests/since_upstream_propagation.rs`'s
/// `keyed_seed_values_flow_through_plan_since_upstream` proves at the
/// assembly level, driven here through the real `smelt` binary.
fn stage_keyed_edge_workspace(parent: &Path) -> PathBuf {
    let root = parent.join("proj");
    write(
        &root,
        "smelt.yml",
        "name: keyed_edge_ws\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\n",
    );
    write(
        &root,
        "models/sources/payments.yml",
        "description: payments\ncolumns:\n- name: user_id\n  type: INTEGER\n\
         - name: amount\n  type: DECIMAL(10,2)\n- name: d\n  type: DATE\n\
         mutation_profile:\n  kind: append_only\n\
         timeseries:\n  partition_column: d\n  event_time_column: d\n  granularity: day\n",
    );
    write(
        &root,
        "models/agg.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
         unique_key: user_id\n---\n\
         SELECT user_id, SUM(amount) AS total\nFROM smelt.sources.payments\n\
         GROUP BY user_id\n",
    );
    write(
        &root,
        "models/downstream.sql",
        "---\nmaterialization: table\ntimeseries:\n  event_time_column: d\n  \
         partition_column: d\n  granularity: day\nrefresh: incremental\n\
         grain: partition\n---\n\
         SELECT DATE '2024-01-01' AS d, user_id, ANY_VALUE(total) AS total \
         FROM smelt.agg GROUP BY user_id\n",
    );
    std::fs::create_dir_all(root.join("target")).unwrap();
    root
}

fn seed_keyed_edge_payments(db_path: &Path) {
    let conn = Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        "CREATE SCHEMA IF NOT EXISTS main;\n\
         CREATE TABLE main.sources_payments (user_id INTEGER, amount DECIMAL(10,2), d DATE);\n\
         INSERT INTO main.sources_payments VALUES \
           (1, 100.00, DATE '2026-01-01'), (1, 50.00, DATE '2026-01-02'), \
           (2, 70.00, DATE '2026-01-01');\n",
    )
    .expect("seed payments source table");
}

fn downstream_total(db_path: &Path, user_id: i64) -> String {
    let conn = Connection::open(db_path).expect("open duckdb");
    let mut stmt = conn
        .prepare("SELECT CAST(total AS VARCHAR) FROM main.downstream WHERE user_id = ?")
        .expect("prepare");
    stmt.query_row([user_id], |row| row.get::<_, String>(0))
        .expect("row for user_id")
}

/// Phase 7 test 6 (`docs/outcomes/20260816-scheduler-delta-signatures/
/// phases/07-plan.md`): end to end over a staged clockless `keyed upsert` →
/// `grain: partition` project. After a build (twice, so `downstream`'s own
/// group-grain sidecar partition over `agg` is seeded before the mutation),
/// mutating one upstream row and running `--since-upstream --source agg
/// --landed <window>` renders the keyed component with the resolved key
/// value in the dirty-set report, and only that key's downstream row
/// reflects the mutation — a second, untouched key's row is unchanged.
#[test]
fn since_upstream_resolves_a_live_non_empty_keyed_restriction() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_keyed_edge_workspace(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_keyed_edge_payments(&db_path);

    // Run 1: creation. Run 2: idempotent rebuild — `downstream`'s own first
    // LIVE key-addressed dispatch, seeding its group-grain sidecar
    // partition over `agg` before any real mutation exists to detect.
    for _ in 0..2 {
        let output = run_smelt(&project_dir, &["--allow-downgrade"]);
        assert!(
            output.status.success(),
            "plain build must succeed: stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(downstream_total(&db_path, 1), "150.00");
    assert_eq!(downstream_total(&db_path, 2), "70.00");

    {
        let conn = Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            "UPDATE main.sources_payments SET amount = 200.00 \
             WHERE user_id = 1 AND amount = 100.00",
        )
        .expect("mutate payments");
    }

    // `agg` is the delta ORIGIN — `--since-upstream` never re-runs it (its
    // landed delta is the output window a completed run already wrote), so
    // its own output must be refreshed by a plain run first.
    {
        let output = run_smelt(&project_dir, &["--select", "agg", "--allow-downgrade"]);
        assert!(
            output.status.success(),
            "agg-only refresh must succeed: stderr={}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let output = run_smelt(
        &project_dir,
        &[
            "--since-upstream",
            "--source",
            "agg",
            "--landed",
            "2026-01-01..2026-01-03",
            "--allow-downgrade",
        ],
    );
    assert!(
        output.status.success(),
        "keyed --since-upstream run must succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("downstream <- agg: keyed"),
        "the dirty-set report must render the keyed component: {stdout}"
    );
    assert!(
        stdout.contains("\"1\""),
        "the dirty-set report must name the resolved key value: {stdout}"
    );

    assert_eq!(
        downstream_total(&db_path, 1),
        "250.00",
        "user 1's row must reflect the mutated contribution (50.00 + 200.00)"
    );
    assert_eq!(
        downstream_total(&db_path, 2),
        "70.00",
        "user 2's row must be unchanged — it was never in the resolved key set"
    );
}

/// Persisted per-source watermark (`docs/specs/run_state.md` §"Per-source
/// watermark"): a plain `smelt run` over a window advances every source
/// `silver` consumes (`bronze`); a following `--since-upstream --source
/// sources.bronze` with **no** `--landed` resolves the delta from that
/// watermark instead of requiring the operator to restate what landed.
#[test]
fn full_run_then_since_upstream_without_landed_propagates() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_workspace(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_sources(&db_path);
    // The watermark is written under `.smelt/`, so this fixture (unlike the
    // rest of this file's `--landed`-driven tests) needs a `state.mode` that
    // persists it — `stage_workspace`'s default `smelt.yml` declares none,
    // which defaults to `stateless` (writes nothing).
    write(
        &project_dir,
        "smelt.yml",
        "name: since_upstream_ws\nversion: 1\npaths:\n  - models\n\
         targets:\n  dev:\n    type: duckdb\n    database: target/dev.duckdb\n    schema: main\n\
         default_materialization: view\nstate:\n  mode: environments\n",
    );

    let full = run_smelt(
        &project_dir,
        &["--start", "2026-01-01", "--end", "2026-01-11"],
    );
    assert!(
        full.status.success(),
        "full run over the seeded window must succeed: stderr={}",
        String::from_utf8_lossy(&full.stderr)
    );

    let output = run_smelt(
        &project_dir,
        &["--since-upstream", "--source", "sources.bronze"],
    );
    assert!(
        output.status.success(),
        "watermark-resolved --since-upstream must succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Dirty set (--since-upstream):"),
        "must print the dirty set before acting: {stdout}"
    );
    assert!(
        stdout.contains("silver <- bronze"),
        "the watermark-resolved delta must propagate to silver: {stdout}"
    );
}

/// A `--source` with neither a paired `--landed` nor a persisted watermark
/// is a named run error — never a silent per-source skip that would quietly
/// under-propagate (`run_state.md` §"Per-source watermark": "the refusal
/// names the missing watermark").
#[test]
fn since_upstream_without_landed_or_watermark_refuses() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_workspace(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_sources(&db_path);

    let output = run_smelt(
        &project_dir,
        &["--since-upstream", "--source", "sources.bronze"],
    );
    assert!(
        !output.status.success(),
        "must refuse when neither --landed nor a persisted watermark exists"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("bronze"),
        "error must name the source: {stderr}"
    );
    assert!(
        stderr.contains("watermark"),
        "error must name the missing watermark: {stderr}"
    );
}
