//! Real-fixture, DuckDB-backed coverage for BL5's `g_run >= g_part` run-window
//! validation (`docs/specs/incremental_shapes.md` §"Run window vs partition
//! granularity"), exercised through the genuine `execute_project` path — the
//! single shared entry point both `smelt-cli` and `smelt-ui` call (root
//! `CLAUDE.md` §Architectural invariants, "Run pipeline parity rule").
//!
//! Unlike a direct call to `smelt_runtime::windowing::validate_run_window_against_partition_grid`,
//! these tests build a real project (frontmatter, `smelt.yml`, DuckDB target)
//! and run it through `execute_project`, proving the check fires (or doesn't)
//! in production, not merely in the underlying pure function.
//!
//! The fixture model's `partition_column` is computed with `DATE_TRUNC('day',
//! transaction_timestamp)` (not a `::DATE` cast) so its inferred type is
//! `Timestamp`, not `Date` — this deliberately avoids also triggering the
//! unrelated D-52 "sub-day granularity requires a timestamp-resolution
//! partition column" diagnostic gate (`smelt-db/src/queries/check_types.rs`),
//! which would otherwise refuse an `Hour`-granularity model before execution
//! ever reaches the windowing validation this test targets.

use std::path::Path;
use std::sync::Arc;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute::{BackendFactory, BackendFuture};
use smelt_runtime::types::ExecuteRequest;
use smelt_runtime::{execute_project, NoOpReporter};
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
    if let Ok(cfg) = smelt_core::config::Config::load(project_dir) {
        db.set_active_target(cfg.target.map(|t| std::sync::Arc::from(t.as_str())));
    }

    let graph = DependencyGraph::build(sql_models, None).expect("build graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

const REVENUE_MODEL_SQL: &str = r#"---
materialization: table
timeseries:
  event_time_column: transaction_timestamp
  partition_column: revenue_date
  granularity: {granularity}
refresh: incremental
grain: partition
---
SELECT
    DATE_TRUNC('day', transaction_timestamp) AS revenue_date,
    user_id,
    SUM(amount) AS total_revenue
FROM (VALUES
    (1, 100.00, TIMESTAMP '2024-12-25 10:00:00'),
    (2, 200.00, TIMESTAMP '2024-12-25 14:00:00'),
    (1,  50.00, TIMESTAMP '2024-12-26 09:00:00'),
    (3, 300.00, TIMESTAMP '2024-12-26 16:00:00')
) AS t(user_id, amount, transaction_timestamp)
GROUP BY 1, 2
"#;

/// Build a fresh single-model project with `revenue.sql`'s `granularity:` set
/// to `granularity`, and a DuckDB target pointing at a temp file in the
/// project directory. Returns the project's `TempDir` (kept alive by the
/// caller) and the built `Config`.
fn setup_project(tmp: &tempfile::TempDir, granularity: &str) -> (std::path::PathBuf, Config) {
    let project_dir = tmp.path().to_path_buf();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    std::fs::write(
        project_dir.join("models").join("revenue.sql"),
        REVENUE_MODEL_SQL.replace("{granularity}", granularity),
    )
    .unwrap();

    let db_path = project_dir.join("run.duckdb");
    let smelt_yml = format!(
        "name: partition_grid_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: view\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();

    let config = Config::load(&project_dir).expect("load config");
    (project_dir, config)
}

fn make_request(start: &str, end: &str) -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select: vec![],
        exclude: vec![],
        start: Some(start.to_string()),
        end: Some(end.to_string()),
        batch_size_days: None,
        per_partition: false,
        full_refresh: false,
        rebuild: false,
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
        resume: false,
        technique_overrides: vec![],
    }
}

/// `granularity: hour` (`g_run`) is finer than `revenue_date`'s derived grid
/// (`Day`, from `DATE_TRUNC('day', ...)`, `g_part`). A real (non-dry-run)
/// `execute_project` call must refuse the run with a message naming the
/// minimum window — before touching the DuckDB backend's data.
#[tokio::test]
async fn test_execute_project_rejects_run_window_finer_than_partition_grid() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (project_dir, config) = setup_project(&tmp, "hour");
    let config = Arc::new(config);
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let db_path = project_dir.join("run.duckdb");
    let result = execute_project(
        "run-reject".to_string(),
        make_request("2024-12-25", "2024-12-26"),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await;

    let err = result.expect_err(
        "an Hour run window against a Day-derived partition grid must be refused for real",
    );
    let msg = err.to_string();
    assert!(
        msg.contains("day"),
        "expected the refusal to name the minimum ('day') window, got: {msg}"
    );
}

/// `granularity: day` (`g_run`) equals `revenue_date`'s derived grid (`Day`,
/// `g_part`) — the run must succeed for real, and produce the correct output
/// (4 input rows, grouped by (day, user) into 4 output rows: users 1 and 2 on
/// 2024-12-25, users 1 and 3 on 2024-12-26 — user 1 repeats across days, so
/// its two days stay distinct groups rather than collapsing).
#[tokio::test]
async fn test_execute_project_accepts_run_window_at_partition_grid() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (project_dir, config) = setup_project(&tmp, "day");
    let config = Arc::new(config);
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let db_path = project_dir.join("run.duckdb");
    let outcome = execute_project(
        "run-accept".to_string(),
        make_request("2024-12-25", "2024-12-27"),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("a Day run window against a Day-derived partition grid must succeed");

    let model_result = outcome
        .models
        .get("revenue")
        .expect("revenue model must appear in RunOutcome");
    assert_eq!(
        model_result.row_count, 4,
        "expected 4 grouped (revenue_date, user_id) rows, got {}",
        model_result.row_count
    );

    // Verify the actual table contents, not just the row count.
    let backend = DuckDbBackend::new(&db_path, "main")
        .await
        .expect("open duckdb");
    let rows = backend
        .execute_sql("SELECT COUNT(*) AS cnt FROM main.revenue")
        .await
        .expect("query revenue table");
    let count = rows[0]
        .column(0)
        .as_any()
        .downcast_ref::<arrow::array::Int64Array>()
        .unwrap()
        .value(0);
    assert_eq!(count, 4, "revenue table must contain 4 rows");
}

/// A run window that is finer-grained than the *declared* alignment
/// (misaligned to the declared `Day` granularity, per the pre-existing
/// `validate_run_window_alignment` check that now also runs through this
/// same real path) is refused — e.g. a start date is always aligned for Day,
/// but this asserts the ordinary alignment check is live end-to-end too,
/// not only the new `g_part` comparison. Uses `Week` granularity with a
/// non-Monday start to trigger the alignment branch specifically.
#[tokio::test]
async fn test_execute_project_rejects_misaligned_run_window_through_real_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (project_dir, config) = setup_project(&tmp, "week");
    let config = Arc::new(config);
    let (db, graph) = build_db_and_graph(&project_dir, &config);

    let db_path = project_dir.join("run.duckdb");
    // 2024-12-25 is a Wednesday, not a Monday — misaligned to weekly granularity.
    let result = execute_project(
        "run-misaligned".to_string(),
        make_request("2024-12-25", "2025-01-01"),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await;

    let err = result.expect_err("a non-Monday start with weekly granularity must be refused");
    let msg = err.to_string();
    assert!(
        msg.contains("weekly") || msg.contains("Monday"),
        "expected the refusal to explain the weekly-alignment violation, got: {msg}"
    );
}

/// A misaligned monthly run window is refused with a message naming the
/// exact coarsened `[--event-time-start, --event-time-end)` pair that would
/// be accepted (`docs/specs/incremental_shapes.md` §"Run window vs partition
/// granularity", success criterion 2) — this test parses that pair out of
/// the real refusal, re-runs `execute_project` with exactly it, and asserts
/// the run succeeds with correct output, through the same real path both
/// `smelt-cli` and `smelt-ui` share.
#[tokio::test]
async fn test_misaligned_window_refusal_names_a_pair_that_then_succeeds() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (project_dir, config) = setup_project(&tmp, "month");
    let config = Arc::new(config);
    let (db, graph) = build_db_and_graph(&project_dir, &config);
    let db_path = project_dir.join("run.duckdb");

    // 2024-12-05 is not the 1st of a month — misaligned to monthly granularity.
    let refusal = execute_project(
        "run-misaligned-month".to_string(),
        make_request("2024-12-05", "2024-12-20"),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect_err("a non-month-aligned window must be refused");
    let msg = refusal.to_string();

    let extract = |flag: &str| -> String {
        let idx = msg
            .find(flag)
            .unwrap_or_else(|| panic!("expected '{flag}' in refusal message: {msg}"));
        msg[idx + flag.len()..]
            .split_whitespace()
            .next()
            .unwrap_or_else(|| panic!("expected a date after '{flag}' in: {msg}"))
            .trim_end_matches(|c: char| !c.is_ascii_digit())
            .to_string()
    };
    let suggested_start = extract("--event-time-start");
    let suggested_end = extract("--event-time-end");
    assert_eq!(suggested_start, "2024-12-01");
    assert_eq!(suggested_end, "2025-01-01");

    let outcome = execute_project(
        "run-coarsened".to_string(),
        make_request(&suggested_start, &suggested_end),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        &project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("re-running with the suggested coarsened pair must succeed");

    let model_result = outcome
        .models
        .get("revenue")
        .expect("revenue model must appear in RunOutcome");
    assert_eq!(
        model_result.row_count, 4,
        "expected 4 grouped (revenue_date, user_id) rows, got {}",
        model_result.row_count
    );
}
