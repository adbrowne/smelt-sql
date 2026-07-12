//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Real-fixture, DuckDB-backed coverage for `docs/specs/batched_models.md`
//! §"Window independence and self-referential models": an `Ordered`
//! (convergent self-edge) model whose `partition_column` is *derived* and
//! skews away from its driving date column gets the same Form B write-window
//! rebase as a window-independent model — a run requesting only day D also
//! rewrites the earlier, skew-reached partitions, and the batches within that
//! rebased range still build strictly sequentially, in temporal order (the
//! self-edge's own `Ordered` requirement, unaffected by this composition).
//!
//! Construct: `sessions_like.session_start_date` reads its own prior
//! partition (`bal.session_start_date >= e.session_start_date - INTERVAL '1
//! day' AND bal.session_start_date < e.session_start_date` —
//! `window_independence`'s own `Ordered` shape, and — not incidentally — the
//! exact false-positive skew-anchor shape `docs/specs/batched_models.md`
//! warns about, since the self-referenced column shares the model's own
//! `partition_column` name) AND accumulates the current partition's events.
//! Separately, a genuine Form B relation
//! (`e.event_date BETWEEN e.session_start_date AND e.session_start_date +
//! INTERVAL '1 day'`) declares that an event dated one day after its assigned
//! `session_start_date` still belongs to that earlier partition — a real
//! 1-day skew, anchored on the *driving source's own* column, not the
//! self-edge.

use std::path::Path;

use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject, SqlCapturingReporter};

fn stage_project(project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();
    std::fs::write(
        project_dir.join("models/sources/events.yml"),
        "description: property-discovery source.\n\
         mutation_profile: append_only\n\
         columns:\n\
         \x20\x20- name: event_date\n    type: DATE\n\
         \x20\x20- name: session_start_date\n    type: DATE\n\
         \x20\x20- name: amt\n    type: DOUBLE\n",
    )
    .unwrap();

    let model_sql = r#"---
timeseries:
  event_time_column: event_date
  partition_column: session_start_date
  granularity: day
refresh: incremental
grain: partition
batched:
  unique_key: [session_start_date]
---
SELECT
  e.session_start_date AS session_start_date,
  COALESCE(bal.total, 0) + SUM(e.amt) AS total
FROM smelt.sources.events e
LEFT JOIN smelt.sessions_like bal
  ON bal.session_start_date >= e.session_start_date - INTERVAL '1 day'
 AND bal.session_start_date < e.session_start_date
WHERE e.event_date BETWEEN e.session_start_date AND e.session_start_date + INTERVAL '1 day'
GROUP BY e.session_start_date, bal.total
"#;
    std::fs::write(project_dir.join("models/sessions_like.sql"), model_sql).unwrap();

    let smelt_yml = format!(
        "name: property_discovery\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

/// Seed the source events (three partition-days' worth, with two events
/// skewed one day later than their assigned `session_start_date`) and a
/// bootstrap opening row dated one day before the first real partition — the
/// same self-referential-model bootstrap sidestep
/// `crates/smelt-runtime/tests/self_referential_ordered_backfill.rs` and
/// `crates/smelt-cli/tests/e2e/two_layer_clamp.rs` use (DuckDB refuses `CREATE
/// TABLE ... AS SELECT` self-referencing itself for a model's very first
/// partition).
fn seed_sources(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE TABLE main.sources_events (event_date DATE, session_start_date DATE, amt DOUBLE);
        INSERT INTO main.sources_events VALUES
            (DATE '2024-01-01', DATE '2024-01-01', 10.0),
            (DATE '2024-01-02', DATE '2024-01-01', 5.0),
            (DATE '2024-01-02', DATE '2024-01-02', 20.0),
            (DATE '2024-01-03', DATE '2024-01-02', 7.0),
            (DATE '2024-01-03', DATE '2024-01-03', 1.0);
        CREATE TABLE main.sessions_like (session_start_date DATE, total DOUBLE);
        INSERT INTO main.sessions_like VALUES (DATE '2023-12-31', 0.0);
        "#,
    )
    .expect("seed sources");
}

fn fetch_total(conn: &duckdb::Connection, date: &str) -> f64 {
    conn.query_row(
        &format!("SELECT total FROM main.sessions_like WHERE session_start_date = DATE '{date}'"),
        [],
        |row| row.get(0),
    )
    .unwrap_or_else(|e| panic!("expected a sessions_like row for {date}: {e}"))
}

async fn run_range(project: &LinkCProject, run_id: &str, start: &str, end: &str) {
    let mut request = base_request("dev");
    request.start = Some(start.to_string());
    request.end = Some(end.to_string());
    project
        .run_quiet(run_id, request)
        .await
        .unwrap_or_else(|e| panic!("run {run_id} [{start}, {end}) must succeed: {e}"));
}

/// The from-scratch oracle: three strictly-sequential single-day runs,
/// D1 → D2 → D3, each necessarily executing the self-edge against the
/// PRIOR day's already-committed row.
async fn build_from_scratch(project: &LinkCProject) {
    run_range(project, "scratch-d1", "2024-01-01", "2024-01-02").await;
    run_range(project, "scratch-d2", "2024-01-02", "2024-01-03").await;
    run_range(project, "scratch-d3", "2024-01-03", "2024-01-04").await;
}

#[tokio::test]
async fn ordered_self_ref_run_rewrites_skewed_prior_partitions() {
    // ── Project A: the from-scratch oracle ──────────────────────────────
    let tmp_a = tempfile::TempDir::new().expect("tempdir a");
    let project_dir_a = tmp_a.path().to_path_buf();
    let db_path_a = project_dir_a.join("dev.duckdb");
    stage_project(&project_dir_a, &db_path_a);
    seed_sources(&db_path_a);
    let project_a = LinkCProject::load(project_dir_a, db_path_a.clone()).expect("load project a");
    build_from_scratch(&project_a).await;

    let (oracle_d1, oracle_d2, oracle_d3) = {
        let conn = project_a.connect().expect("connect a");
        (
            fetch_total(&conn, "2024-01-01"),
            fetch_total(&conn, "2024-01-02"),
            fetch_total(&conn, "2024-01-03"),
        )
    };
    // Hand-derived: D1 = 0(bootstrap) + 10 + 5(skewed in from D2) = 15.
    // D2 = 15(bal) + 20 + 7(skewed in from D3) = 42.
    // D3 = 42(bal) + 1 = 43.
    assert_eq!(
        oracle_d1, 15.0,
        "oracle D1: bootstrap 0 + own 10 + skewed-in 5"
    );
    assert_eq!(oracle_d2, 42.0, "oracle D2: bal 15 + own 20 + skewed-in 7");
    assert_eq!(oracle_d3, 43.0, "oracle D3: bal 42 + own 1");

    // ── Project B: D1 committed by its own run, THEN a single "day 3
    // alone" request — must rebase to cover D2 (the skew-reached partition)
    // as well as D3, in strict temporal order, converging to the SAME
    // values as the from-scratch oracle.
    let tmp_b = tempfile::TempDir::new().expect("tempdir b");
    let project_dir_b = tmp_b.path().to_path_buf();
    let db_path_b = project_dir_b.join("dev.duckdb");
    stage_project(&project_dir_b, &db_path_b);
    seed_sources(&db_path_b);
    let project_b = LinkCProject::load(project_dir_b, db_path_b.clone()).expect("load project b");

    run_range(&project_b, "b-d1", "2024-01-01", "2024-01-02").await;

    let reporter = SqlCapturingReporter::new();
    let mut request = base_request("dev");
    request.start = Some("2024-01-03".to_string());
    request.end = Some("2024-01-04".to_string());
    project_b
        .run("b-day3-alone", request, &reporter)
        .await
        .unwrap_or_else(|e| panic!("day-3-alone run must succeed: {e}"));

    // Rebase composed with the Ordered driver: the requested single day
    // (D3) widens by the derived 1-day skew to cover D2 as well — two
    // batches, not one.
    let compiled = reporter.sql_for("sessions_like");
    assert_eq!(
        compiled.len(),
        2,
        "a single-day request must rebase into two sequential batches \
         (the skew-reached prior partition D2, then D3 itself), got: {compiled:?}"
    );
    assert!(
        compiled[0].contains("2024-01-02"),
        "the first executed batch must be the skew-reached prior partition D2: {}",
        compiled[0]
    );
    assert!(
        compiled[1].contains("2024-01-03"),
        "the second executed batch must be D3, built strictly after D2: {}",
        compiled[1]
    );

    let conn = project_b.connect().expect("connect b");
    let (rebuilt_d1, rebuilt_d2, rebuilt_d3) = (
        fetch_total(&conn, "2024-01-01"),
        fetch_total(&conn, "2024-01-02"),
        fetch_total(&conn, "2024-01-03"),
    );

    assert_eq!(
        rebuilt_d1, oracle_d1,
        "D1 was untouched by the day-3-alone run (outside the rebased window) \
         and must still match the oracle"
    );
    assert_eq!(
        rebuilt_d2, oracle_d2,
        "D2 must have been rewritten by the rebased day-3-alone run, reading \
         D1's already-committed row, and match the from-scratch oracle"
    );
    assert_eq!(
        rebuilt_d3, oracle_d3,
        "D3 must read D2's FRESHLY rewritten row (strict temporal order \
         within the rebased run) and match the from-scratch oracle"
    );
}
