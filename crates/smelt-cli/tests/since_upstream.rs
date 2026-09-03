#![cfg(feature = "duckdb")]
//! MP15 (`docs/plans/20260707-maintenance-plan-impl.md`): `smelt run
//! --since-upstream` — forward propagation from caller-declared per-source
//! deltas (`incremental_models.md` §CLI, §"The graph layer"). Per the ratified
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
    // Same-partition self-read (`d = d`, no backward reach) is circular, not
    // convergent — refused by the shared derivation
    // (`window_independence.rs`'s `self_edge_bound_days`) at the graph
    // layer's own call site, carrying that derivation's reason rather than
    // `propagate.rs`'s later generic "no derivable backward bound" text.
    assert!(
        stderr.contains("current partition") || stderr.contains("circular"),
        "{stderr}"
    );
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

/// Seed a single `_smelt_observed_delta` row directly (bypassing an actual
/// conditional write, mirroring `seed_composed`'s direct-population
/// approach — the observed-delta read side under test doesn't care how the
/// row got there).
fn seed_observed_delta(
    db_path: &Path,
    model: &str,
    window_start: &str,
    window_end: &str,
    changed_keys: &[&str],
    partitions: &[&str],
) {
    let conn = Connection::open(db_path).expect("open duckdb");
    let ddl = smelt_state::ddl_duckdb::generate_observed_delta_table_ddl("main");
    conn.execute_batch(&ddl)
        .expect("create observed-delta table");
    let render = |vals: &[&str]| -> String {
        if vals.is_empty() {
            "[]::VARCHAR[]".to_string()
        } else {
            format!(
                "[{}]",
                vals.iter()
                    .map(|v| format!("'{v}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    };
    let insert = format!(
        "INSERT INTO main._smelt_observed_delta \
         (model_name, window_start, window_end, changed_keys, partitions) \
         VALUES ('{model}', '{window_start}', '{window_end}', {}, {})",
        render(changed_keys),
        render(partitions),
    );
    conn.execute_batch(&insert)
        .expect("seed observed-delta row");
}

/// Phase 15 (`docs/outcomes/20260815-definition-delta-migrate/phases/
/// 15-plan.md`): a recorded observed delta narrows `--since-upstream`'s
/// dirty set to exactly the recorded partitions, instead of the whole
/// declared `--landed` window — using the same locality-admitted composed
/// origin fixture as `composed_model_address_landed_delta_propagates`,
/// but with a much WIDER declared window than the recorded delta covers.
#[test]
fn recorded_observed_delta_narrows_the_dirty_set() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_composed_origin_workspace(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_composed(&db_path);
    seed_observed_delta(
        &db_path,
        "composed",
        "2026-01-01",
        "2026-01-11",
        &["5"],
        &["2026-01-05"],
    );

    let output = run_smelt(
        &project_dir,
        &[
            "--since-upstream",
            "--source",
            "composed",
            "--landed",
            "2026-01-01..2026-01-11",
        ],
    );
    assert!(
        output.status.success(),
        "narrowed since-upstream run must succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        gold_dates_composed(&db_path),
        vec!["2026-01-05".to_string()],
        "the recorded delta must narrow the dirty set to exactly the recorded partition, not \
         the whole 10-day declared window"
    );
}

/// A **present-and-empty** recorded observed delta propagates nothing —
/// the CLI's own end-to-end leg of `empty_observed_delta_schedules_zero_
/// downstream_regions` (`crates/smelt-runtime/tests/
/// since_upstream_propagation.rs`).
#[test]
fn present_and_empty_observed_delta_propagates_nothing() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_composed_origin_workspace(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_composed(&db_path);
    seed_observed_delta(&db_path, "composed", "2026-01-01", "2026-01-11", &[], &[]);

    let output = run_smelt(
        &project_dir,
        &[
            "--since-upstream",
            "--source",
            "composed",
            "--landed",
            "2026-01-01..2026-01-11",
        ],
    );
    assert!(
        output.status.success(),
        "an empty recorded delta must not be a refusal: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("propagated nothing"),
        "a present-and-empty recorded delta must propagate nothing: {stderr}"
    );
}

/// Unchanged baseline: with no recorded observed delta at all (absent),
/// `--since-upstream` falls back to the declared `--landed` window exactly
/// as before this phase (widen-never-narrow) — the CLI's own end-to-end
/// pin of the "absent" branch, using the same wide 10-day window the
/// narrowing test above uses so the two are directly comparable.
#[test]
fn absent_observed_delta_falls_back_to_the_declared_window() {
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
            "2026-01-01..2026-01-11",
        ],
    );
    assert!(
        output.status.success(),
        "baseline since-upstream run must succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let mut expected: Vec<String> = (1..=10).map(|d| format!("2026-01-{:02}", d)).collect();
    expected.sort();
    assert_eq!(
        gold_dates_composed(&db_path),
        expected,
        "with no recorded delta, the whole declared window must be dirtied, unwidened and \
         unnarrowed"
    );
}

/// Phase 21 (`docs/outcomes/20260815-definition-delta-migrate/outcome.md`):
/// a chain of two bare `grain: key` models — `keyed_a` (a real declared
/// `timeseries:` source feeds it, admitting a `KeyedUpsert` output-delta
/// shape) feeding `keyed_b`, itself also admitting `KeyedUpsert` — runs end
/// to end under `--since-upstream`, past the graph layer's keyed dirt-set
/// cascade (`smelt_logical::maintenance::propagate::propagate`) and the
/// runtime's consumption of it (`smelt_runtime::propagation::
/// plan_since_upstream_with_observed_deltas`). `keyed_a` is the delta origin
/// (already materialized — its own completed run wrote it); `keyed_b` is
/// scheduled as a keyed (whole-table) run and actually built by this
/// invocation, exercising the same "node dirtied only through the keyed
/// channel" cascade `bare_keyed_model_with_readers_is_scheduled`
/// (`crates/smelt-runtime/tests/since_upstream_propagation.rs`) pins at the
/// assembly level.
fn stage_bare_keyed_chain_workspace(parent: &Path) -> PathBuf {
    let root = parent.join("proj");
    write(
        &root,
        "smelt.yml",
        "name: bare_keyed_chain_ws\nversion: 1\npaths:\n  - models\n\
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
        "models/keyed_a.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n---\n\
         SELECT user_id, SUM(amount) AS total FROM smelt.sources.payments GROUP BY user_id\n",
    );
    write(
        &root,
        "models/keyed_b.sql",
        "---\nmaterialization: table\nrefresh: incremental\ngrain: key\n\
         unique_key: [user_id]\n---\n\
         SELECT user_id, ANY_VALUE(total) AS grand_total FROM smelt.keyed_a GROUP BY user_id\n",
    );
    std::fs::create_dir_all(root.join("target")).unwrap();
    root
}

/// Pre-populate `main.keyed_a` — the delta origin's own completed run.
fn seed_keyed_a(db_path: &Path) {
    let conn = Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        "CREATE SCHEMA IF NOT EXISTS main;\n\
         CREATE TABLE main.keyed_a (user_id INTEGER, total DECIMAL(10,2));\n\
         INSERT INTO main.keyed_a VALUES (1, 10.0), (2, 20.0), (3, 30.0);\n",
    )
    .expect("seed keyed_a");
}

#[test]
fn since_upstream_over_a_bare_keyed_chain_runs_end_to_end() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_bare_keyed_chain_workspace(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_keyed_a(&db_path);

    let output = run_smelt(
        &project_dir,
        &[
            "--since-upstream",
            "--source",
            "keyed_a",
            "--landed",
            "2026-01-03..2026-01-04",
        ],
    );
    assert!(
        output.status.success(),
        "the bare keyed chain must run end to end: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Dirty set (--since-upstream):"),
        "must print the dirty set before acting: {stdout}"
    );
    assert!(
        stdout.contains("keyed_b <-(keyed) keyed_a"),
        "the dirty set must show the keyed edge: {stdout}"
    );
    assert!(
        stdout.contains("RUN keyed_b: keyed"),
        "keyed_b must be scheduled as a keyed (whole-table) run: {stdout}"
    );
    assert!(
        !stdout.contains("RUN keyed_a"),
        "the delta origin must not be re-run: {stdout}"
    );

    let conn = Connection::open(&db_path).expect("open duckdb");
    let keyed_b_rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM main.keyed_b", [], |row| row.get(0))
        .expect("keyed_b must have been built");
    assert!(keyed_b_rows > 0, "keyed_b must be rebuilt with rows");
}

/// Phase 23 (`docs/outcomes/20260815-definition-delta-migrate`): `--select`
/// scoping for `--since-upstream` intersects the propagated plan with the
/// ordinary CLI selector instead of ignoring it. All four tests reuse
/// `stage_model_chain` (`sources.bronze -> silver -> gold`) with a
/// `--source bronze --landed ...` delta, which dirties both `silver` and
/// `gold` transitively (`silver -> gold` is a maintained-model edge).
fn seed_bronze(db_path: &Path) {
    let conn = Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        "CREATE SCHEMA IF NOT EXISTS main;\n\
         CREATE TABLE main.sources_bronze (id INTEGER, d DATE);\n\
         INSERT INTO main.sources_bronze \
           SELECT i, DATE '2026-01-01' + CAST(i - 1 AS INTEGER) FROM range(1, 11) t(i);\n",
    )
    .expect("seed bronze table");
}

fn table_exists(db_path: &Path, table: &str) -> bool {
    let conn = Connection::open(db_path).expect("open duckdb");
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'main' AND table_name = ?",
            [table],
            |row| row.get(0),
        )
        .expect("query information_schema");
    count > 0
}

/// `--select silver` (no `+`) with a `bronze` delta that dirties both
/// `silver` and `gold`: only `silver` executes; `gold` is suppressed and
/// its table is never created (untouched).
#[test]
fn select_narrows_the_since_upstream_run_set() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_model_chain(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_bronze(&db_path);

    let output = run_smelt(
        &project_dir,
        &[
            "--since-upstream",
            "--source",
            "sources.bronze",
            "--landed",
            "2026-01-03..2026-01-04",
            "--select",
            "silver",
        ],
    );
    assert!(
        output.status.success(),
        "narrowed since-upstream run must succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("SUPPRESSED (not selected): gold"),
        "the report must name gold as suppressed: {stdout}"
    );
    assert!(table_exists(&db_path, "silver"), "silver must have run");
    assert!(
        !table_exists(&db_path, "gold"),
        "gold must stay untouched (never created) when deselected"
    );
}

/// `--select +gold` pulls `silver` in via the upstream operator — both
/// models in the dirty chain execute.
#[test]
fn select_with_upstream_operator_keeps_the_dirty_chain() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_model_chain(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_bronze(&db_path);

    let output = run_smelt(
        &project_dir,
        &[
            "--since-upstream",
            "--source",
            "sources.bronze",
            "--landed",
            "2026-01-03..2026-01-04",
            "--select",
            "+gold",
        ],
    );
    assert!(
        output.status.success(),
        "+gold since-upstream run must succeed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(table_exists(&db_path, "silver"), "silver must have run");
    assert!(table_exists(&db_path, "gold"), "gold must have run");
}

/// `--select gold` alone, with a `bronze` delta dirtying both `silver` and
/// `gold`, drops gold's dirty direct upstream (`silver`) from the
/// selection — refused fail-loud rather than run `gold` against a stale
/// `silver`.
#[test]
fn select_dropping_a_dirty_upstream_refuses() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_model_chain(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_bronze(&db_path);

    let output = run_smelt(
        &project_dir,
        &[
            "--since-upstream",
            "--source",
            "sources.bronze",
            "--landed",
            "2026-01-03..2026-01-04",
            "--select",
            "gold",
        ],
    );
    assert!(
        !output.status.success(),
        "dropping a dirty upstream from the selection must refuse"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("gold"), "error must name gold: {stderr}");
    assert!(
        stderr.contains("silver"),
        "error must name silver: {stderr}"
    );
    assert!(!stderr.contains("panicked at"), "{stderr}");
}

/// A selector matching no model in the propagated plan is a quiet no-op —
/// exit `0`, nothing executed, per `cli.md`'s "no models matched" contract.
#[test]
fn select_matching_nothing_is_a_quiet_no_op() {
    let tmp = TempDir::new().unwrap();
    let project_dir = stage_model_chain(tmp.path());
    let db_path = project_dir.join("target/dev.duckdb");
    seed_bronze(&db_path);

    let output = run_smelt(
        &project_dir,
        &[
            "--since-upstream",
            "--source",
            "sources.bronze",
            "--landed",
            "2026-01-03..2026-01-04",
            "--select",
            "tag:does-not-exist",
        ],
    );
    assert!(
        output.status.success(),
        "an empty intersection must exit 0: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no models matched the selector(s)"),
        "{stderr}"
    );
    assert!(!table_exists(&db_path, "silver"), "nothing must execute");
    assert!(!table_exists(&db_path, "gold"), "nothing must execute");
}

/// `--since-upstream` over the whole, unfiltered `examples/web_analytics`
/// workspace (no `--select`) completes under `--dry-run`: phase 22's
/// day-unrolled self-edge (`silver.sessions_chained`, then propagated to its
/// only reader `silver.events_enriched`) schedules an open-ended
/// `[start, →)` run, which phase 24's `resolve_run_window` resolves to a
/// finite window before `execute_project` — rather than dying on
/// `parse_run_window`'s "Both start and end must be provided together"
/// guard. Flagship end-to-end gate for
/// `docs/outcomes/20260815-definition-delta-migrate`'s phase 24.
#[test]
fn web_analytics_whole_workspace_since_upstream_dry_run_completes() {
    let web_analytics_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
        .join("web_analytics");
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("dev.duckdb");

    let output = Command::new(smelt_bin())
        .arg("run")
        .arg("--since-upstream")
        .arg("--source")
        .arg("sources.raw.events")
        .arg("--landed")
        .arg("2026-03-22..2026-03-23")
        .arg("--dry-run")
        .arg("--project-dir")
        .arg(&web_analytics_dir)
        .arg("--database")
        .arg(&db_path)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run --since-upstream`: {e}"));

    assert!(
        output.status.success(),
        "whole-workspace --since-upstream --dry-run must complete: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !combined.contains("Both start and end"),
        "the open-ended self-edge frontier must resolve to a finite run window, not refuse: {combined}"
    );
    assert!(
        combined.contains("[2026-03-22, \u{2192})") || combined.contains("[2026-03-20, \u{2192})"),
        "the printed dirty-set report must still show the open-ended form for the self-edge \
         chain: {combined}"
    );
    // Phase 24b: `silver.device_user_edges` — a bare-keyed model whose
    // upstream (`silver.events_deduped`) is a clocked, differently-keyed
    // model edge — is scheduled via the keyed dirt channel, and its own
    // key-addressed model-edge cell now admits (`grain-over-upstream`
    // discovery) rather than silently having no cell to dispatch at real
    // run time.
    assert!(
        combined.contains("RUN silver.device_user_edges: keyed"),
        "silver.device_user_edges must appear in the RUN set: {combined}"
    );
}

/// The same whole-workspace run under `--verbose` names the resolved closed
/// window alongside the open-ended form in its per-run log line.
#[test]
fn web_analytics_open_ended_run_logs_the_resolved_window() {
    let web_analytics_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
        .join("web_analytics");
    let tmp = TempDir::new().unwrap();
    let db_path = tmp.path().join("dev.duckdb");

    let output = Command::new(smelt_bin())
        .arg("run")
        .arg("--since-upstream")
        .arg("--source")
        .arg("sources.raw.events")
        .arg("--landed")
        .arg("2026-03-22..2026-03-23")
        .arg("--dry-run")
        .arg("--verbose")
        .arg("--project-dir")
        .arg(&web_analytics_dir)
        .arg("--database")
        .arg(&db_path)
        .env("RUST_LOG", "info")
        .env("NO_COLOR", "1")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run --since-upstream`: {e}"));

    assert!(
        output.status.success(),
        "whole-workspace --since-upstream --dry-run must complete: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let chained_line = stdout
        .lines()
        .find(|l| l.contains("running silver.sessions_chained"))
        .unwrap_or_else(|| panic!("no log line for silver.sessions_chained: {stdout}"));
    assert!(
        chained_line.contains("\u{2192}) \u{2192} ["),
        "the log line must name both the open-ended and resolved-closed forms: {chained_line}"
    );
}
