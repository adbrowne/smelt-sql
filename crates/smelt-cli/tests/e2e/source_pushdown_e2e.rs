#![cfg(feature = "duckdb")]
//! End-to-end test: source-filter pushdown for incremental batch reads (BUG-073).
//!
//! Verifies that when a `timeseries:`-declaring source is read by an incremental
//! model, each batch's compiled SQL contains the source-read filter
//! (`WHERE partition_col >= run_start AND partition_col < run_end`) produced by
//! `inject_source_filters`, not just the outer time filter on the model's own
//! partition column.
//!
//! RED before Phase 3: the incremental batch loop calls `inject_time_filter`
//! (which filters the model output) but does NOT call `inject_source_filters`
//! (which narrows the source reads). After Phase 3 the batch loop calls both.
//!
//! The test also verifies that the run result is correct (source pushdown is an
//! optimization, not a semantics change): the row count for a [D, D+1) window
//! matches what a full-scan source would produce.

use std::path::Path;
use std::sync::{Arc, Mutex};

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute::{BackendFactory, BackendFuture};
use smelt_runtime::execute_project;
use smelt_runtime::reporter::RunReporter;
use smelt_runtime::types::ExecuteRequest;
use tokio_util::sync::CancellationToken;

// ── SQL capturing reporter ────────────────────────────────────────────────────

/// Reporter that captures compiled SQL for each model batch.
#[derive(Default)]
struct SqlCapturingReporter {
    sqls: Mutex<Vec<String>>,
}

impl SqlCapturingReporter {
    fn captured_sqls(&self) -> Vec<String> {
        self.sqls.lock().unwrap().clone()
    }
}

impl RunReporter for SqlCapturingReporter {
    fn model_compiled(&self, _run_id: &str, _model: &str, sql: &str) {
        self.sqls.lock().unwrap().push(sql.to_string());
    }
}

// ── DuckDB backend factory ────────────────────────────────────────────────────

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

// ── Fixture helpers ───────────────────────────────────────────────────────────

/// Create a hermetic project with:
/// - `models/sources/events.yml`: source with `timeseries:` (partition_column: event_date)
/// - `models/daily_count.sql`: incremental model reading smelt.sources.events
fn stage_project(project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();

    // Source YAML with timeseries declaration (Phase 1 feature).
    // Per-entity source YAML format: flat document with description, columns, timeseries.
    // File path models/sources/events.yml → smelt ref smelt.sources.events → DB: main.sources_events
    let source_yml = r#"description: Raw events partitioned by day.
columns:
  - name: event_date
    type: DATE
  - name: user_id
    type: INTEGER
  - name: event_type
    type: VARCHAR
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
"#;
    std::fs::write(project_dir.join("models/sources/events.yml"), source_yml).unwrap();

    // Incremental model reading from the source.
    let model_sql = r#"---
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
refresh: incremental
grain: partition
---
SELECT event_date, COUNT(*) AS cnt FROM smelt.sources.events GROUP BY event_date
"#;
    std::fs::write(project_dir.join("models/daily_count.sql"), model_sql).unwrap();

    // smelt.yml
    let smelt_yml = format!(
        "name: pushdown_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

/// Seed the DuckDB with source data for 2 days in `main.sources_events`.
/// smelt maps source address `sources.events` → `main.sources_events` by default.
fn seed_events(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_events AS
        SELECT * FROM (VALUES
            (DATE '2024-01-01', 1, 'login'),
            (DATE '2024-01-01', 2, 'purchase'),
            (DATE '2024-01-01', 3, 'view'),
            (DATE '2024-01-02', 1, 'logout'),
            (DATE '2024-01-02', 4, 'login')
        ) AS t(event_date, user_id, event_type);
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

// ── The test ──────────────────────────────────────────────────────────────────

/// Verify that the compiled SQL for an incremental batch contains a source-read
/// filter when the source has a `timeseries:` declaration.
///
/// RED before Phase 3: the batch loop does not call `inject_source_filters`, so
/// the compiled SQL contains only the outer time-filter on `event_date` (the
/// model's partition column), not a subquery narrowing the source scan.
///
/// GREEN after Phase 3: the compiled SQL wraps `main.sources_events` (the resolved
/// source name) in a subquery with `WHERE event_date >= '2024-01-01' AND event_date < '2024-01-02'`.
#[tokio::test]
async fn incremental_run_pushes_source_filter() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let reporter = SqlCapturingReporter::default();
    let request = ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: Some("2024-01-01".to_string()),
        end: Some("2024-01-02".to_string()),
        batch_size_days: None,
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
        retry_max: None,
        retry_backoff_ms: None,
    };

    let outcome = execute_project(
        "pushdown-test".to_string(),
        request,
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect("execute_project must succeed");

    // At least one model must have completed.
    assert!(
        !outcome.models.is_empty(),
        "expected at least one model in outcome; got: {:?}",
        outcome.models.keys().collect::<Vec<_>>()
    );

    // The reporter must have captured at least one compiled SQL.
    let sqls = reporter.captured_sqls();
    assert!(
        !sqls.is_empty(),
        "SqlCapturingReporter must have captured at least one compiled SQL"
    );

    // Check the compiled SQL for source filter injection.
    // After Phase 3: the batch loop calls inject_source_filters BEFORE compilation.
    // inject_source_filters wraps smelt.sources.events as:
    //   (SELECT * FROM smelt.sources.events WHERE event_date >= 'D' AND event_date < 'D+1')
    // Then the SQL compiler resolves smelt.sources.events → main.sources_events, giving:
    //   (SELECT * FROM main.sources_events WHERE event_date >= 'D' AND event_date < 'D+1')
    // Before Phase 3 the source is referenced directly as:
    //   main.sources_events
    // (with no wrapping subquery).
    let all_sqls = sqls.join("\n---\n");

    // The compiled SQL must contain the source wrapped in a subquery with a WHERE filter.
    // This is the key assertion: `(SELECT * FROM main.sources_events WHERE` would only
    // appear if inject_source_filters ran before compilation.
    assert!(
        all_sqls.contains("(SELECT * FROM main.sources_events WHERE"),
        "compiled SQL must contain source-filter subquery \
         '(SELECT * FROM main.sources_events WHERE'; \
         this fires before Phase 3 (source filter not injected into batch loop). \
         Compiled SQLs:\n{all_sqls}"
    );

    // The source filter must be narrowed to the run window [2024-01-01, 2024-01-02).
    assert!(
        all_sqls.contains("event_date >= '2024-01-01'"),
        "compiled SQL must contain run-window start filter 'event_date >= \\'2024-01-01\\''; \
         compiled SQLs:\n{all_sqls}"
    );

    // B0 (unified pushdown-depth walk): `daily_count` is a transparent
    // single-source model (no lookback — `derive_model_bounds` yields
    // `Bounded(event_date, 0, 0)`), so the source-level filter alone is both
    // the scan-pruning filter and the exact output clamp. The outer
    // `inject_time_filter` wrap is redundant here and must be skipped —
    // assert there is exactly *one* occurrence of the start-of-window
    // filter, not a duplicate outer-clamp copy of the same bound.
    let start_filter_occurrences = all_sqls.matches("event_date >= '2024-01-01'").count();
    assert_eq!(
        start_filter_occurrences, 1,
        "transparent single-source slice must emit exactly one filter (source-level \
         only, no outer wrap); got {start_filter_occurrences} occurrences in:\n{all_sqls}"
    );

    // Verify correctness: the run should produce 1 partition (2024-01-01 only,
    // since we requested [2024-01-01, 2024-01-02)).
    let daily_count_record = outcome
        .models
        .get("daily_count")
        .or_else(|| outcome.models.values().next())
        .expect("daily_count model must be in outcome");
    assert!(
        daily_count_record.row_count > 0,
        "daily_count must have produced at least 1 row; got 0"
    );
}

/// Equivalence assertion: pushdown must not change the query results.
/// Running [D, D+1) with source pushdown must produce the same rows as
/// a full-refresh query filtered to the same partition.
#[tokio::test]
async fn source_pushdown_preserves_correctness() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_project(&project_dir, &db_path);
    seed_events(&db_path).expect("seed events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    // Run [2024-01-01, 2024-01-03) — covers both seeded days.
    let request = ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: Some("2024-01-01".to_string()),
        end: Some("2024-01-03".to_string()),
        batch_size_days: None,
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
        retry_max: None,
        retry_backoff_ms: None,
    };

    let outcome = execute_project(
        "pushdown-correctness-test".to_string(),
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

    // The 2-day seed has 3 rows on day 1 and 2 rows on day 2 → 2 distinct partitions.
    let daily_count_record = outcome
        .models
        .get("daily_count")
        .or_else(|| outcome.models.values().next())
        .expect("daily_count model must be in outcome");

    assert_eq!(
        daily_count_record.row_count, 2,
        "daily_count must produce 2 rows (one per day) for [2024-01-01, 2024-01-03); \
         got: {}",
        daily_count_record.row_count
    );
}

// ── B1: subquery/CTE construct consumer (real-fixture, matches full refresh) ──

/// Stage a project with one timeseries source (`events2`) and a
/// `refresh: batched` model whose body is a `WITH`-clause CTE — the
/// subquery/CTE consumer added in this phase.
fn stage_cte_project(project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();

    let source_yml = r#"description: Events for CTE-bodied model test
columns:
  - name: event_date
    type: DATE
  - name: user_id
    type: INTEGER
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
"#;
    std::fs::write(project_dir.join("models/sources/events2.yml"), source_yml).unwrap();

    let model_sql = r#"---
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
refresh: incremental
grain: partition
batched:
  safety_overrides:
    allow_subqueries: true
---
WITH staged AS (
    SELECT event_date, user_id FROM smelt.sources.events2
)
SELECT event_date, COUNT(*) AS cnt FROM staged GROUP BY event_date
"#;
    std::fs::write(project_dir.join("models/cte_daily.sql"), model_sql).unwrap();

    let smelt_yml = format!(
        "name: cte_pushdown_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn seed_cte_events(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_events2 AS
        SELECT * FROM (VALUES
            (DATE '2024-01-01', 1),
            (DATE '2024-01-01', 2),
            (DATE '2024-01-02', 3)
        ) AS t(event_date, user_id);
        "#,
    )?;
    Ok(())
}

/// A `WITH`-clause CTE whose body directly projects the model's partition
/// column from a real timeseries source traces `Traceable`, pushing the
/// filter into the CTE's underlying source; an incremental run over a
/// sub-window matches full refresh over the same window.
#[tokio::test]
async fn cte_body_pushes_filter_and_matches_full_refresh() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_cte_project(&project_dir, &db_path);
    seed_cte_events(&db_path).expect("seed cte events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let reporter = SqlCapturingReporter::default();
    let incremental_request = ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: Some("2024-01-01".to_string()),
        end: Some("2024-01-02".to_string()),
        batch_size_days: None,
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
        retry_max: None,
        retry_backoff_ms: None,
    };
    let incremental_outcome = execute_project(
        "cte-pushdown-test".to_string(),
        incremental_request,
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect("incremental execute_project must succeed");

    let all_sqls = reporter.captured_sqls().join("\n---\n");
    assert!(
        all_sqls.contains("(SELECT * FROM main.sources_events2 WHERE"),
        "CTE's underlying source must get a pushdown filter; compiled SQLs:\n{all_sqls}"
    );

    let cte_record = incremental_outcome
        .models
        .get("cte_daily")
        .or_else(|| incremental_outcome.models.values().next())
        .expect("cte_daily model must be in outcome");
    assert_eq!(
        cte_record.row_count, 1,
        "incremental run over day 1 must produce exactly 1 day-row; got {}",
        cte_record.row_count
    );

    let full_request = ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: Some("2024-01-01".to_string()),
        end: Some("2024-01-02".to_string()),
        batch_size_days: None,
        per_partition: false,
        full_refresh: true,
        dry_run: false,
        enforce_safety: false,
        allow_column_removal: false,
        allow_full_refresh: true,
        ephemeral_seed_ctes: vec![],
        run_checks: false,
        checks: vec![],
        jobs: None,
        retry_max: None,
        retry_backoff_ms: None,
    };
    let full_outcome = execute_project(
        "cte-pushdown-full-refresh-test".to_string(),
        full_request,
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
    .expect("full-refresh execute_project must succeed");
    let full_record = full_outcome
        .models
        .get("cte_daily")
        .or_else(|| full_outcome.models.values().next())
        .expect("cte_daily model must be in full-refresh outcome");
    assert_eq!(
        full_record.row_count, cte_record.row_count,
        "full refresh over the same window must match the incremental run's row count \
         (per-partition equivalence)"
    );
}

// ── B1: UNION ALL construct consumer (real-fixture, matches full refresh) ────
//
// This fixture was previously blocked by a pre-existing, unconditional
// diagnostic — `rule_diagnostics::check_event_time_injectable`'s "Case 1: set
// operation" arm (`RuleDiagnosticCode::EventTimeColumnNotVisibleAtOuterSelect`)
// rejected *any* model with a declared `event_time_column` whose SQL had a set
// operation (`UNION`/`INTERSECT`/`EXCEPT`), before this phase's per-branch
// tracing (`rules::incremental::trace_union_branches`) ever ran. That
// diagnostic is now relaxed (`rule_diagnostics::check_union_all_injectable`)
// to reuse the same per-branch trace the pushdown-scoping walk uses: only
// UNION ALL is eligible, and only when every branch's projection of
// `event_time_column` traces `Traceable` — the model below is exactly that
// case, so it now reaches `execute_project` and this test exercises the real
// end-to-end path.

/// Stage a project with two timeseries sources (`events_a`, `events_b`) and a
/// `refresh: batched` model whose body is a UNION ALL over both — the
/// per-branch construct consumer added in this phase. Each branch directly
/// projects the model's declared `event_time_column`/`partition_column`
/// (`event_date`) from its own source, so both branches trace `Traceable`.
fn stage_union_all_project(project_dir: &Path, db_path: &Path) {
    std::fs::create_dir_all(project_dir.join("models/sources")).unwrap();

    let source_yml = |desc: &str| {
        format!(
            "description: {desc}\n\
             columns:\n\
             \x20 - name: event_date\n\
             \x20   type: DATE\n\
             \x20 - name: user_id\n\
             \x20   type: INTEGER\n\
             timeseries:\n\
             \x20 event_time_column: event_date\n\
             \x20 partition_column: event_date\n\
             \x20 granularity: day\n"
        )
    };
    std::fs::write(
        project_dir.join("models/sources/events_a.yml"),
        source_yml("Events source A for UNION ALL fixture"),
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models/sources/events_b.yml"),
        source_yml("Events source B for UNION ALL fixture"),
    )
    .unwrap();

    let model_sql = r#"---
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
refresh: incremental
grain: partition
---
SELECT event_date, user_id FROM smelt.sources.events_a
UNION ALL
SELECT event_date, user_id FROM smelt.sources.events_b
"#;
    std::fs::write(project_dir.join("models/all_events.sql"), model_sql).unwrap();

    let smelt_yml = format!(
        "name: union_pushdown_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

/// Seed both sources with 2 days of data, distinct row counts per day per
/// source, so an incorrectly-scoped branch (e.g. one source's full history
/// leaking into a single-day incremental run) would change the observed row
/// count.
fn seed_union_all_events(db_path: &Path) -> anyhow::Result<()> {
    let conn = duckdb::Connection::open(db_path)?;
    conn.execute_batch(
        r#"
        CREATE SCHEMA IF NOT EXISTS main;
        CREATE OR REPLACE TABLE main.sources_events_a AS
        SELECT * FROM (VALUES
            (DATE '2024-01-01', 1),
            (DATE '2024-01-01', 2),
            (DATE '2024-01-02', 3)
        ) AS t(event_date, user_id);
        CREATE OR REPLACE TABLE main.sources_events_b AS
        SELECT * FROM (VALUES
            (DATE '2024-01-01', 10),
            (DATE '2024-01-02', 11),
            (DATE '2024-01-02', 12)
        ) AS t(event_date, user_id);
        "#,
    )?;
    Ok(())
}

/// A UNION ALL model whose branches each directly project the partition
/// column from a distinct real timeseries source traces `Traceable` per
/// branch, licensing per-source pushdown; an incremental run over a
/// sub-window matches full refresh over the same window.
#[tokio::test]
async fn union_all_pushes_filter_and_matches_full_refresh() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let project_dir = tmp.path().to_path_buf();
    let db_path = project_dir.join("dev.duckdb");

    stage_union_all_project(&project_dir, &db_path);
    seed_union_all_events(&db_path).expect("seed union-all events");

    let config = Arc::new(Config::load(&project_dir).expect("load config"));
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let reporter = SqlCapturingReporter::default();
    let incremental_request = ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: Some("2024-01-01".to_string()),
        end: Some("2024-01-02".to_string()),
        batch_size_days: None,
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
        retry_max: None,
        retry_backoff_ms: None,
    };
    let incremental_outcome = execute_project(
        "union-pushdown-test".to_string(),
        incremental_request,
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect("incremental execute_project must succeed");

    let all_sqls = reporter.captured_sqls().join("\n---\n");
    assert!(
        all_sqls.contains("(SELECT * FROM main.sources_events_a WHERE"),
        "events_a must get a per-branch pushdown filter; compiled SQLs:\n{all_sqls}"
    );
    assert!(
        all_sqls.contains("(SELECT * FROM main.sources_events_b WHERE"),
        "events_b must get a per-branch pushdown filter; compiled SQLs:\n{all_sqls}"
    );

    let incremental_record = incremental_outcome
        .models
        .get("all_events")
        .or_else(|| incremental_outcome.models.values().next())
        .expect("all_events model must be in outcome");
    // Day 1 only: 2 rows from events_a + 1 row from events_b.
    assert_eq!(
        incremental_record.row_count, 3,
        "incremental run over day 1 must produce exactly 3 rows (2 from events_a, \
         1 from events_b); got {}",
        incremental_record.row_count
    );

    let full_request = ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: Some("2024-01-01".to_string()),
        end: Some("2024-01-02".to_string()),
        batch_size_days: None,
        per_partition: false,
        full_refresh: true,
        dry_run: false,
        enforce_safety: false,
        allow_column_removal: false,
        allow_full_refresh: true,
        ephemeral_seed_ctes: vec![],
        run_checks: false,
        checks: vec![],
        jobs: None,
        retry_max: None,
        retry_backoff_ms: None,
    };
    let full_outcome = execute_project(
        "union-pushdown-full-refresh-test".to_string(),
        full_request,
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
    .expect("full-refresh execute_project must succeed");
    let full_record = full_outcome
        .models
        .get("all_events")
        .or_else(|| full_outcome.models.values().next())
        .expect("all_events model must be in full-refresh outcome");
    assert_eq!(
        full_record.row_count, incremental_record.row_count,
        "full refresh over the same window must match the incremental run's row count \
         (per-partition equivalence)"
    );
}
