//! `EXPERIMENTAL(property-discovery): disposable`
//!
//! Cell `G-08` (`docs/research/20260705-property-discovery-loop.md` §4).
//!
//! Construct: a **self-referential batched model** maintaining a running
//! balance (`docs/specs/incremental_models.md` §"Window independence and
//! self-referential models") — `running_balance.d` reads its own prior
//! partition (`bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d`,
//! `window_independence`'s own `Ordered` shape) and folds in the current
//! day's transaction total. Combiner-algebra (Link 0): additive over a
//! prefix, but **unbounded-forward footprint** (paper §7) — a change to one
//! partition's stored value invalidates every later partition's stored
//! trajectory, not just its own.
//!
//! Hypothesis (revised from the catalog's original "fold-delta (end-state) /
//! stored trajectory" framing once the actual construct was identified):
//! smelt's batched maintenance is recompute-region (`G-03`/`G-04`/`G-07`'s
//! established finding — `DELETE [start,end)` + fresh `INSERT`, no partial
//! aggregate state folded). For THIS partition alone, recompute-region reads
//! the self-reference's CURRENT stored value, so a backfill of exactly the
//! mutated partition is correct. But recompute-region has no cascade: it
//! does not know or re-run any LATER partition whose own stored `balance`
//! was computed from the pre-mutation value. Predicted CONDITIONAL: the
//! trajectory is correct only under the discipline "every backfill of
//! partition `p` is followed by a backfill of every partition `> p` in
//! temporal order" — smelt does not enforce or automate that cascade.
//!
//! Reachability (design §2.1 N4): a transaction is appended into an
//! ALREADY-PROCESSED day (day 1) between runs — a pre-populated source would
//! be seen identically by both full-refresh and the maintained trajectory
//! and never exercise the cascade question at all.

use std::path::Path;

use crate::shapes::{running_balance_self_ref, ModelShape};
use smelt_maintenance_testkit::link_c_harness::{base_request, LinkCProject};
use smelt_maintenance_testkit::oracle::multiset_equal;

fn stage_project(shape: &ModelShape, project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();
    std::fs::write(
        project_dir.join(format!("models/{}.sql", shape.name)),
        shape.sql,
    )
    .unwrap();

    let cols: String = shape
        .source_columns
        .iter()
        .map(|c| format!("  - name: {}\n    type: {}\n", c.name, c.ty))
        .collect();
    let source_yml = format!(
        "description: property-discovery source.\nmutation_profile: append_only\ncolumns:\n{cols}"
    );
    std::fs::write(
        project_dir.join(format!("models/sources/{}.yml", shape.source)),
        source_yml,
    )
    .unwrap();

    let smelt_yml = format!(
        "name: property_discovery\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn seed_sources(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE TABLE main.sources_transactions (d DATE, amt DOUBLE);
        INSERT INTO main.sources_transactions VALUES
            (DATE '2024-01-01', 10.0),
            (DATE '2024-01-02', 5.0),
            (DATE '2024-01-03', 1.0);
        CREATE TABLE main.running_balance (d DATE, balance DOUBLE);
        "#,
    )
    .expect("seed sources");
}

/// Same fixture as [`seed_sources`], but WITHOUT pre-creating the
/// self-referential model's own target table
/// (`main.running_balance`) — proves the framework bootstraps it
/// (`docs/specs/incremental_models.md` §"First-run and backfill" —
/// "First-run bootstrap for a self-referential model"), not the test
/// fixture.
fn seed_sources_no_target_table(db_path: &Path) {
    let conn = duckdb::Connection::open(db_path).expect("open duckdb");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE TABLE main.sources_transactions (d DATE, amt DOUBLE);
        INSERT INTO main.sources_transactions VALUES
            (DATE '2024-01-01', 10.0),
            (DATE '2024-01-02', 5.0),
            (DATE '2024-01-03', 1.0);
        "#,
    )
    .expect("seed sources (no pre-seeded target table)");
}

/// Full-refresh oracle: the running balance re-derived from scratch over the
/// CURRENT full contents of the source table, up to and including `date` —
/// no smelt compilation, no stored self-reference, no derived filter ("full-
/// refresh over the source state at step k", design N3).
fn full_refresh_balance(conn: &duckdb::Connection, date: &str) -> f64 {
    conn.query_row(
        &format!("SELECT SUM(amt) FROM main.sources_transactions WHERE d <= DATE '{date}'"),
        [],
        |row| row.get(0),
    )
    .expect("full-refresh oracle query")
}

fn maintained_balance(conn: &duckdb::Connection, date: &str) -> f64 {
    conn.query_row(
        &format!("SELECT balance FROM main.running_balance WHERE d = DATE '{date}'"),
        [],
        |row| row.get(0),
    )
    .expect("maintained-table read")
}

async fn run_day(project: &LinkCProject, run_id: &str, day: &str, next_day: &str) {
    let mut request = base_request("dev");
    request.start = Some(day.to_string());
    request.end = Some(next_day.to_string());
    project
        .run_quiet(run_id, request)
        .await
        .unwrap_or_else(|e| panic!("run {run_id} [{day}, {next_day}) must succeed: {e}"));
}

#[tokio::test]
async fn late_transaction_into_an_already_processed_partition_requires_a_downstream_cascade() {
    let shape = running_balance_self_ref();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&shape, &project_dir, &db_path);
    seed_sources(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    // Build the initial trajectory sequentially, in temporal order (the
    // `Ordered` self-edge's own requirement) — day1=10, day2=15, day3=16.
    run_day(&project, "run-1", "2024-01-01", "2024-01-02").await;
    run_day(&project, "run-2", "2024-01-02", "2024-01-03").await;
    run_day(&project, "run-3", "2024-01-03", "2024-01-04").await;

    {
        let conn = project.connect().expect("connect after initial trajectory");
        assert_eq!(maintained_balance(&conn, "2024-01-01"), 10.0);
        assert_eq!(maintained_balance(&conn, "2024-01-02"), 15.0);
        assert_eq!(maintained_balance(&conn, "2024-01-03"), 16.0);
    }

    // Between runs: append a LATE transaction into the already-processed
    // 2024-01-01 partition — the true balance from that day forward is now
    // 110 / 115 / 116, but only day1's OWN stored row is stale so far.
    {
        let conn = project.connect().expect("connect for late append");
        conn.execute(
            "INSERT INTO main.sources_transactions VALUES (DATE '2024-01-01', 100.0)",
            [],
        )
        .expect("append late transaction into already-processed partition");
    }

    // Backfill EXACTLY the mutated partition (day1 alone) — recompute-region
    // re-reads the current source contents for that partition, so day1's own
    // stored value must self-correct.
    run_day(&project, "run-4-backfill-day1", "2024-01-01", "2024-01-02").await;

    let (day1_after_local_backfill, day2_after_local_backfill, day3_after_local_backfill) = {
        let conn = project.connect().expect("connect after day1-only backfill");
        (
            maintained_balance(&conn, "2024-01-01"),
            maintained_balance(&conn, "2024-01-02"),
            maintained_balance(&conn, "2024-01-03"),
        )
    };
    assert_eq!(
        day1_after_local_backfill, 110.0,
        "recompute-region must re-read the source's CURRENT contents for the \
         partition it is explicitly asked to rebuild"
    );
    assert_eq!(
        day2_after_local_backfill, 15.0,
        "EXPECTED (the CONDITIONAL finding, not a bug): a backfill of day1 alone \
         does not cascade to day2's stored trajectory value — recompute-region has \
         no mechanism to discover or re-run a downstream partition whose stored \
         self-reference read is now stale"
    );
    assert_eq!(
        day3_after_local_backfill, 16.0,
        "EXPECTED (same CONDITIONAL finding): day3 is stale for the same reason"
    );

    // The condition under which the trajectory DOES match full-refresh:
    // every partition at or after the mutated one is backfilled, in temporal
    // order (the self-edge's own ordering requirement).
    run_day(&project, "run-5-cascade-day2", "2024-01-02", "2024-01-03").await;
    run_day(&project, "run-6-cascade-day3", "2024-01-03", "2024-01-04").await;

    let conn = project.connect().expect("connect after downstream cascade");
    for date in ["2024-01-01", "2024-01-02", "2024-01-03"] {
        assert!(
            multiset_equal(
                &conn,
                &format!("SELECT balance FROM main.running_balance WHERE d = DATE '{date}'"),
                &format!("SELECT {} AS balance", full_refresh_balance(&conn, date)),
            ),
            "after backfilling the mutated partition AND every downstream partition \
             in temporal order, the trajectory must match full-refresh for {date}"
        );
    }
    assert_eq!(maintained_balance(&conn, "2024-01-01"), 110.0);
    assert_eq!(maintained_balance(&conn, "2024-01-02"), 115.0);
    assert_eq!(maintained_balance(&conn, "2024-01-03"), 116.0);
}

/// Framework bootstrap (`docs/specs/incremental_models.md` §"First-run and
/// backfill" — "First-run bootstrap for a self-referential model"): a
/// self-referential model builds from scratch with NO pre-seeded target
/// table — `main.running_balance` does not exist before the first `smelt
/// run` — and still produces the same sequential trajectory a manually
/// seeded table would. A second run over the same range is idempotent
/// (multiset-equal, not merely non-erroring).
#[tokio::test]
async fn builds_from_scratch_with_no_preseeded_target_table() {
    let shape = running_balance_self_ref();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&shape, &project_dir, &db_path);
    seed_sources_no_target_table(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");

    // The target table must not exist yet — otherwise this test is not
    // actually exercising the bootstrap path.
    {
        let conn = project.connect().expect("connect before first run");
        let exists: bool = conn
            .query_row(
                "SELECT count(*) > 0 FROM information_schema.tables \
                 WHERE table_schema = 'main' AND table_name = 'running_balance'",
                [],
                |row| row.get(0),
            )
            .expect("check target table absence");
        assert!(
            !exists,
            "precondition: main.running_balance must not exist before the first run"
        );
    }

    // Build the trajectory sequentially, in temporal order (the `Ordered`
    // self-edge's own requirement), exactly like the pre-seeded fixture
    // above — the ONLY difference is the target table starts out absent.
    run_day(&project, "scratch-run-1", "2024-01-01", "2024-01-02").await;
    run_day(&project, "scratch-run-2", "2024-01-02", "2024-01-03").await;
    run_day(&project, "scratch-run-3", "2024-01-03", "2024-01-04").await;

    {
        let conn = project.connect().expect("connect after from-scratch build");
        assert_eq!(
            maintained_balance(&conn, "2024-01-01"),
            full_refresh_balance(&conn, "2024-01-01"),
        );
        assert_eq!(maintained_balance(&conn, "2024-01-01"), 10.0);
        assert_eq!(maintained_balance(&conn, "2024-01-02"), 15.0);
        assert_eq!(maintained_balance(&conn, "2024-01-03"), 16.0);
    }

    // Idempotence: re-running the same three days again must reproduce the
    // exact same trajectory, not accumulate a second empty-bootstrap or
    // double-count rows.
    run_day(&project, "scratch-run-1-repeat", "2024-01-01", "2024-01-02").await;
    run_day(&project, "scratch-run-2-repeat", "2024-01-02", "2024-01-03").await;
    run_day(&project, "scratch-run-3-repeat", "2024-01-03", "2024-01-04").await;

    let conn = project.connect().expect("connect after idempotent re-run");
    for date in ["2024-01-01", "2024-01-02", "2024-01-03"] {
        assert!(
            multiset_equal(
                &conn,
                &format!("SELECT balance FROM main.running_balance WHERE d = DATE '{date}'"),
                &format!("SELECT {} AS balance", full_refresh_balance(&conn, date)),
            ),
            "re-running the same range must reproduce the full-refresh trajectory for {date}"
        );
    }
    assert_eq!(maintained_balance(&conn, "2024-01-01"), 10.0);
    assert_eq!(maintained_balance(&conn, "2024-01-02"), 15.0);
    assert_eq!(maintained_balance(&conn, "2024-01-03"), 16.0);
}

/// Layered variant of the from-scratch bootstrap: the same running-balance
/// construct placed under `models/silver/`, self-referenced by its full
/// canonical dot-path (`smelt.silver.running_balance` — the only self-ref
/// spelling that resolves; an unqualified `smelt.running_balance` written
/// inside `silver/` is refused at the diagnostic gate with an
/// `UndefinedModelRef` fix-it naming the canonical path, and would compile
/// to a *different* physical table anyway). Pins that the runtime's shared
/// self-edge predicate (canonical-path equality,
/// `smelt-runtime`'s `is_self_referential`) fires for layered models
/// end-to-end: no pre-seeded target, bootstrap must create
/// `main.silver_running_balance`, and the trajectory must match the
/// sequential expectation.
#[tokio::test]
async fn layered_model_builds_from_scratch_with_canonical_self_ref() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    std::fs::create_dir_all(project_dir.join("models/silver")).unwrap();
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();
    std::fs::write(
        project_dir.join("models/silver/running_balance.sql"),
        r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: incremental
grain: partition
batched:
  unique_key: [d]
---
SELECT d, balance FROM (
  SELECT
    t.d AS d,
    COALESCE(bal.balance, 0) + SUM(t.amt) AS balance
  FROM smelt.sources.transactions t
  LEFT JOIN smelt.silver.running_balance bal
    ON bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d
  GROUP BY t.d, bal.balance
) inner_balance
"#,
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models/sources/transactions.yml"),
        "description: property-discovery source.\nmutation_profile: append_only\ncolumns:\n  - name: d\n    type: DATE\n  - name: amt\n    type: DOUBLE\n",
    )
    .unwrap();
    let smelt_yml = format!(
        "name: property_discovery\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();

    // Source only — no pre-seeded main.silver_running_balance target.
    {
        let conn = duckdb::Connection::open(&db_path).expect("open duckdb");
        conn.execute_batch(
            r#"
            CREATE SCHEMA IF NOT EXISTS main;
            CREATE TABLE main.sources_transactions (d DATE, amt DOUBLE);
            INSERT INTO main.sources_transactions VALUES
                (DATE '2024-01-01', 10.0),
                (DATE '2024-01-02', 5.0);
            "#,
        )
        .expect("seed source (no target table)");
    }

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");
    run_day(&project, "layered-run-1", "2024-01-01", "2024-01-02").await;
    run_day(&project, "layered-run-2", "2024-01-02", "2024-01-03").await;

    let conn = project.connect().expect("connect after layered build");
    let day1: f64 = conn
        .query_row(
            "SELECT balance FROM main.silver_running_balance WHERE d = DATE '2024-01-01'",
            [],
            |row| row.get(0),
        )
        .expect("read layered day-1 balance");
    let day2: f64 = conn
        .query_row(
            "SELECT balance FROM main.silver_running_balance WHERE d = DATE '2024-01-02'",
            [],
            |row| row.get(0),
        )
        .expect("read layered day-2 balance");
    assert_eq!(day1, 10.0);
    assert_eq!(day2, 15.0);
}

/// Fail-loud guard (`architecture.md` §"Fail-loud discipline"): a
/// self-referential model whose output column type cannot be inferred (the
/// balance flows through a function smelt's type inference has no rule
/// for, so it stays `Unknown` at every fixpoint round) must be REFUSED
/// with a diagnostic naming the untypeable column — never bootstrapped into
/// `CREATE TABLE … (balance UNKNOWN)`, which would surface as an opaque
/// engine catalog error (`Type with name UNKNOWN does not exist`).
#[tokio::test]
async fn untypeable_self_ref_column_is_refused_with_a_diagnostic() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();
    // `gamma(...)` is a real DuckDB function smelt's inference has no
    // typing rule for — the column's type stays Unknown through the
    // schema fixpoint.
    std::fs::write(
        project_dir.join("models/running_balance.sql"),
        r#"---
timeseries:
  event_time_column: d
  partition_column: d
  granularity: day
refresh: incremental
grain: partition
batched:
  unique_key: [d]
---
SELECT d, balance FROM (
  SELECT
    t.d AS d,
    gamma(COALESCE(bal.balance, 0) + SUM(t.amt)) AS balance
  FROM smelt.sources.transactions t
  LEFT JOIN smelt.running_balance bal
    ON bal.d >= t.d - INTERVAL '1 day' AND bal.d < t.d
  GROUP BY t.d, bal.balance
) inner_balance
"#,
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models/sources/transactions.yml"),
        "description: property-discovery source.\nmutation_profile: append_only\ncolumns:\n  - name: d\n    type: DATE\n  - name: amt\n    type: DOUBLE\n",
    )
    .unwrap();
    let smelt_yml = format!(
        "name: property_discovery\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();

    seed_sources_no_target_table(&db_path);

    let project = LinkCProject::load(project_dir.clone(), db_path.clone()).expect("load project");
    let mut request = base_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-01-02".to_string());
    let err = project
        .run_quiet("untypeable-run", request)
        .await
        .expect_err("a from-scratch run of an untypeable self-ref model must be refused");
    let msg = err.to_string();
    assert!(
        msg.contains("balance") && msg.contains("could not be inferred"),
        "the refusal must be a diagnostic naming the untypeable column, not an opaque \
         engine catalog error; got: {msg}"
    );
    assert!(
        !msg.contains("Type with name UNKNOWN"),
        "the engine catalog error must never be the failure surface; got: {msg}"
    );
}
