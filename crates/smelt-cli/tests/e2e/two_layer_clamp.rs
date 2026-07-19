//! End-to-end test: the write side of a batched-incremental model must clamp
//! to exactly the output window, never the widened scan window
//! (`docs/specs/model_transforms.md` §Semantics — "Source-filter pushdown and
//! the two clamps"; §Design — "Rejected: auto-widening the write window").
//!
//! Fixture: a model with a genuine 1-day lookback expressed as an interval
//! self-join (`prev_day.prev_date = cur.transaction_date - INTERVAL '1
//! day'`) over a `smelt.ref` source declaring `timeseries:`. Two sequential
//! daily batches are run through the real `execute_project` pipeline in a
//! single call (mirroring `smelt run`'s batch loop in
//! `crates/smelt-runtime/src/execute.rs`).
//!
//! (A window-function lookback, e.g. `LAG`, was deliberately avoided here:
//! `inject_time_filter` appends its clamp as a plain `WHERE` at the *same*
//! SELECT level as the model body, and standard SQL evaluates `WHERE` before
//! window functions — so for a query whose outer SELECT computes a window
//! function directly, that same-level `WHERE` would filter the window
//! function's *input* rows, not just its output, independently of and prior
//! to this fix. A plain join has no such ordering hazard: `WHERE` on the
//! join's result set is a normal post-join row filter, which is exactly the
//! "exact output clamp" this fixture needs to isolate.)
//!
//! The bug this locks down: before the fix, both the output clamp
//! (`inject_time_filter`) and the DELETE partition range used the *wide*
//! `filter_start`/`filter_end` window (the scan's read margin) instead of the
//! narrow `partition_start`/`partition_end` output window. That meant batch 2
//! (day 2) would DELETE *and recompute* day 1's already-correct partition —
//! but using a scan sized for batch 2's own margin (which reaches back only
//! to day 1, not day 0), silently dropping day 1's legitimate lookback
//! context and corrupting its `prev_ts` value.
//!
//! With the fix, the DELETE and output clamp for batch 2 cover only day 2;
//! day 1's partition (written correctly by batch 1, whose own margin reached
//! back to day 0) is left untouched.

use std::path::Path;
use std::sync::Arc;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute::{BackendFactory, BackendFuture};
use smelt_runtime::execute_project;
use smelt_runtime::types::ExecuteRequest;
use tokio_util::sync::CancellationToken;

struct DuckDbBackendFactory {
    db_path: std::path::PathBuf,
}

impl BackendFactory for DuckDbBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        let path = self.db_path.clone();
        let schema = target_config.schema.clone();
        Box::pin(async move {
            let backend = DuckDbBackend::new(&path, &schema)
                .await
                .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
            Ok(Box::new(backend) as Box<dyn Backend>)
        })
    }
}

/// Stage a project with:
/// - `models/sources/transactions.yml`: a `timeseries:`-declaring source
///   (partition_column: transaction_date).
/// - `models/user_activity.sql`: a `refresh: batched` model that joins the
///   source to itself, offset by `INTERVAL '1 day'`, to look up each row's
///   prior-day transaction — the Form B interval-join lookback pattern from
///   `docs/specs/incremental_models.md`.
fn stage_project(project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();

    let source_yml = r#"description: Raw transactions, timeseries by transaction_date.
columns:
  - name: transaction_date
    type: DATE
  - name: user_id
    type: INTEGER
  - name: transaction_ts
    type: TIMESTAMP
timeseries:
  event_time_column: transaction_date
  partition_column: transaction_date
  granularity: day
"#;
    std::fs::write(
        project_dir.join("models/sources/transactions.yml"),
        source_yml,
    )
    .unwrap();

    let model_sql = r#"---
timeseries:
  event_time_column: transaction_date
  partition_column: transaction_date
  granularity: day
refresh: incremental
grain: partition
---
WITH cur AS (
    SELECT transaction_date, user_id, transaction_ts
    FROM smelt.sources.transactions
),
prev_day AS (
    SELECT transaction_date AS prev_date, user_id, transaction_ts AS prev_ts
    FROM smelt.sources.transactions
)
SELECT
    cur.transaction_date,
    cur.user_id,
    cur.transaction_ts,
    prev_day.prev_ts
FROM cur
LEFT JOIN prev_day
    ON prev_day.user_id = cur.user_id
    AND prev_day.prev_date = cur.transaction_date - INTERVAL '1 day'
WHERE cur.transaction_date >= cur.transaction_date - INTERVAL '1 day'
"#;
    std::fs::write(project_dir.join("models/user_activity.sql"), model_sql).unwrap();

    let smelt_yml = format!(
        "name: two_layer_clamp_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

/// Seed `main.sources_transactions` with one row per day for user 1, spanning
/// a day *before* the requested run window (2023-12-31) through the last
/// requested day (2024-01-02). The 2023-12-31 row exists only to give
/// 2024-01-01's prev_ts a legitimate one-day-back predecessor — it is never
/// itself part of the requested `[2024-01-01, 2024-01-03)` output window.
fn seed_transactions(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_transactions AS
        SELECT * FROM (VALUES
            (DATE '2023-12-31', 1, TIMESTAMP '2023-12-31 10:00:00'),
            (DATE '2024-01-01', 1, TIMESTAMP '2024-01-01 09:00:00'),
            (DATE '2024-01-02', 1, TIMESTAMP '2024-01-02 09:00:00')
        ) AS t(transaction_date, user_id, transaction_ts);
        "#,
    )?;
    Ok(())
}

fn build_db_and_graph(
    project_dir: &Path,
    config: &Config,
) -> (
    Arc<tokio::sync::Mutex<smelt_db::Database>>,
    Arc<tokio::sync::Mutex<DependencyGraph>>,
) {
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery.discover_models().expect("discover_models");

    let mut db = smelt_db::Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let source_files: Vec<_> = sql_models
        .iter()
        .map(|m| db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf()))
        .collect();
    db.set_workspace(source_files, vec![project]);

    let graph = DependencyGraph::build(sql_models, None).expect("build graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

/// Fetch `prev_ts` (as a formatted string, or "NULL") for `user_activity`'s
/// row on the given `transaction_date`.
fn fetch_prev_ts(db_path: &Path, transaction_date: &str) -> anyhow::Result<Option<String>> {
    let conn = duckdb::Connection::open(db_path)?;
    let mut stmt = conn.prepare(
        "SELECT CAST(prev_ts AS VARCHAR) FROM main.user_activity \
         WHERE transaction_date = ?::DATE AND user_id = 1",
    )?;
    let mut rows = stmt.query([transaction_date])?;
    if let Some(row) = rows.next()? {
        let val: Option<String> = row.get(0)?;
        Ok(val)
    } else {
        panic!("expected a row for transaction_date = {transaction_date}, found none");
    }
}

/// Two sequential daily batches through the real `execute_project` pipeline:
/// batch 1 (2024-01-01) correctly resolves prev_ts back to 2023-12-31; batch 2
/// (2024-01-02) must not re-touch batch 1's partition. The write window must
/// equal the output window for every batch.
#[tokio::test]
async fn batched_write_clamp_does_not_widen_to_scan_margin() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_transactions(&db_path).expect("seed transactions");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    // [2024-01-01, 2024-01-03) split into 2 one-day batches — the batch loop
    // in execute.rs processes both sequentially within this single call.
    let request = ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: Some("2024-01-01".to_string()),
        end: Some("2024-01-03".to_string()),
        batch_size_days: Some(1),
        per_partition: false,
        full_refresh: false,
        dry_run: false,
        enforce_safety: false,
        allow_column_removal: false,
        allow_full_refresh: false,
        ephemeral_seed_ctes: vec![],
        run_checks: false,
        checks: vec![],
        jobs: None,
    };

    let outcome = execute_project(
        "two-layer-clamp-test".to_string(),
        request,
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &smelt_runtime::NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("execute_project must succeed");

    assert!(
        !outcome.models.is_empty(),
        "expected at least one model in outcome"
    );

    // 2024-01-01's row must retain its correct lookback value: the interval
    // join resolves prev_ts to 2023-12-31's transaction_ts. Under the pre-fix
    // bug, batch 2 (2024-01-02) would DELETE and recompute this row using a
    // scan that only reaches back to 2024-01-01 (its own margin), losing the
    // 2023-12-31 context and leaving prev_ts NULL.
    let prev_ts_jan_1 =
        fetch_prev_ts(&db_path, "2024-01-01").expect("fetch prev_ts for 2024-01-01");
    assert_eq!(
        prev_ts_jan_1.as_deref(),
        Some("2023-12-31 10:00:00"),
        "2024-01-01's prev_ts must still resolve to 2023-12-31's transaction_ts after \
         batch 2 runs — the write clamp must not have re-touched this partition \
         with an insufficiently widened scan; got {prev_ts_jan_1:?}"
    );

    // 2024-01-02's row resolves to 2024-01-01's transaction_ts (its own
    // correctly-scoped batch).
    let prev_ts_jan_2 =
        fetch_prev_ts(&db_path, "2024-01-02").expect("fetch prev_ts for 2024-01-02");
    assert_eq!(
        prev_ts_jan_2.as_deref(),
        Some("2024-01-01 09:00:00"),
        "2024-01-02's prev_ts must resolve to 2024-01-01's transaction_ts; got {prev_ts_jan_2:?}"
    );

    // No row should exist for 2023-12-31 — the write window never extends
    // past the requested output range, even though the scan reads it.
    let conn = duckdb::Connection::open(&db_path).expect("open db");
    let leaked: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM main.user_activity WHERE transaction_date = DATE '2023-12-31'",
            [],
            |row| row.get(0),
        )
        .expect("query leaked-row count");
    assert_eq!(
        leaked, 0,
        "the write window must never extend to the scan's margin \
         (2023-12-31 is read for context but must never be written)"
    );
}

/// Negative control: hand-derive the pre-fix (buggy) SQL batch 2 would have
/// compiled — output clamp AND DELETE widened to `[filter_start, filter_end)`
/// (`2024-01-01` to `2024-01-03`) instead of the narrow `[2024-01-02,
/// 2024-01-03)` run window — and prove it diverges from the correct,
/// narrowly-clamped result. This is the fixture-validity check: if this test
/// stopped diverging, the fixture would no longer exercise the bug the fix
/// addresses.
#[tokio::test]
async fn pre_fix_wide_clamp_would_have_corrupted_the_prior_partition() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("dev.duckdb");

    seed_transactions(&db_path).expect("seed transactions");

    let conn = duckdb::Connection::open(&db_path).expect("open db");
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        "#,
    )
    .unwrap();

    // Batch 1 (correct either way): scan widened to [2023-12-30, 2024-01-02),
    // output clamped to [2024-01-01, 2024-01-02).
    conn.execute_batch(
        r#"
        CREATE OR REPLACE TABLE main.user_activity AS
        WITH cur AS (
            SELECT transaction_date, user_id, transaction_ts
            FROM main.sources_transactions
            WHERE transaction_date >= DATE '2023-12-30' AND transaction_date < DATE '2024-01-02'
        ), prev_day AS (
            SELECT transaction_date AS prev_date, user_id, transaction_ts AS prev_ts
            FROM main.sources_transactions
            WHERE transaction_date >= DATE '2023-12-30' AND transaction_date < DATE '2024-01-02'
        )
        SELECT cur.transaction_date, cur.user_id, cur.transaction_ts, prev_day.prev_ts
        FROM cur
        LEFT JOIN prev_day
            ON prev_day.user_id = cur.user_id
            AND prev_day.prev_date = cur.transaction_date - INTERVAL '1 day'
        WHERE cur.transaction_date >= DATE '2024-01-01' AND cur.transaction_date < DATE '2024-01-02';
        "#,
    )
    .unwrap();

    // Batch 2, PRE-FIX behavior: scan widened relative to batch 2's own
    // margin ([2024-01-01, 2024-01-03) — unaffected by this fix), but the
    // DELETE and output clamp use the WIDE filter_start/filter_end window
    // ([2024-01-01, 2024-01-03)) instead of the narrow run window
    // ([2024-01-02, 2024-01-03)).
    conn.execute_batch(
        r#"
        DELETE FROM main.user_activity
        WHERE transaction_date >= DATE '2024-01-01' AND transaction_date < DATE '2024-01-03';

        INSERT INTO main.user_activity
        WITH cur AS (
            SELECT transaction_date, user_id, transaction_ts
            FROM main.sources_transactions
            WHERE transaction_date >= DATE '2024-01-01' AND transaction_date < DATE '2024-01-03'
        ), prev_day AS (
            SELECT transaction_date AS prev_date, user_id, transaction_ts AS prev_ts
            FROM main.sources_transactions
            WHERE transaction_date >= DATE '2024-01-01' AND transaction_date < DATE '2024-01-03'
        )
        SELECT cur.transaction_date, cur.user_id, cur.transaction_ts, prev_day.prev_ts
        FROM cur
        LEFT JOIN prev_day
            ON prev_day.user_id = cur.user_id
            AND prev_day.prev_date = cur.transaction_date - INTERVAL '1 day'
        WHERE cur.transaction_date >= DATE '2024-01-01' AND cur.transaction_date < DATE '2024-01-03';
        "#,
    )
    .unwrap();

    // Under the pre-fix behavior, 2024-01-01's row was corrupted: recomputed
    // with a scan that doesn't reach back to 2023-12-31, so prev_ts is NULL
    // instead of the correct 2023-12-31 timestamp.
    let prev_ts: Option<String> = conn
        .query_row(
            "SELECT prev_ts FROM main.user_activity WHERE transaction_date = DATE '2024-01-01'",
            [],
            |row| row.get(0),
        )
        .expect("query prev_ts");
    assert!(
        prev_ts.is_none(),
        "fixture-validity check: the pre-fix wide clamp must corrupt \
         2024-01-01's row (NULL prev_ts) — if it now resolves the correct \
         value, this fixture no longer demonstrates the bug the fix \
         addresses; got {prev_ts:?}"
    );
}
