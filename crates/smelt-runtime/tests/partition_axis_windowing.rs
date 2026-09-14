//! Phase 5a tests for the integer partition axis (`docs/specs/timeseries.md`
//! §"Validation rules" rule 9, `docs/specs/incremental_shapes.md` §"The
//! partition grain" rule 8a) — the unit-step integer grid, its
//! `PartitionPoint` arithmetic, run-window validation, and the day-typed
//! widening refusal. Calendar-axis coverage (unchanged behavior) lives in
//! `windowing_parity.rs`; this file only covers the new integer-axis branch
//! and the `PartitionPoint` type itself.

use std::collections::HashMap;

use smelt_core::config::TimeseriesConfig;
use smelt_core::{Granularity, PartitionGrainConfig, PartitionGrainSafetyOverrides};
use smelt_runtime::windowing::{
    compute_incremental_windows, validate_run_window_against_partition_grid, PartitionAxis,
    PartitionPoint,
};
use smelt_runtime::TimeRange;

fn make_ts(event_col: &str, partition_col: &str, granularity: Granularity) -> TimeseriesConfig {
    TimeseriesConfig {
        event_time_column: event_col.to_string(),
        partition_column: partition_col.to_string(),
        granularity,
        week_start: None,
        assert_monotonic: false,
    }
}

fn make_inc() -> PartitionGrainConfig {
    PartitionGrainConfig {
        unique_key: vec![],
        nondeterministic_columns_retired: (),
        safety_overrides: PartitionGrainSafetyOverrides::default(),
    }
}

fn make_range(start: &str, end: &str) -> TimeRange {
    TimeRange {
        start: start.to_string(),
        end: end.to_string(),
        axis: smelt_logical::PartitionAxis::Calendar,
        column_type: smelt_logical::maintenance::emit::PartitionColumnType::Undeclared,
    }
}

fn no_dep_timeseries() -> HashMap<String, (Vec<String>, String)> {
    HashMap::new()
}

// ── PartitionPoint::Display / sql_literal ──────────────────────────────────

#[test]
fn partition_point_display_and_sql_literal() {
    let d = PartitionPoint::Date(chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap());
    assert_eq!(d.to_string(), "2026-01-01");
    assert_eq!(d.sql_literal(), "'2026-01-01'");

    let i = PartitionPoint::Integer(7);
    assert_eq!(i.to_string(), "7");
    assert_eq!(i.sql_literal(), "7");
}

// ── Integer-axis chunking ───────────────────────────────────────────────────

#[test]
fn integer_axis_chunks_by_unit_steps() {
    let sql = "SELECT batch_id, id FROM events";
    let ts = make_ts("event_ts", "batch_id", Granularity::Day);
    let inc = make_inc();
    let range = make_range("1", "4");

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        &range,
        PartitionAxis::Integer,
        None,
        true, // per_partition: one unit per batch
    )
    .expect("integer axis must not be refused");

    assert_eq!(windows.batches.len(), 3, "expected [1,2) [2,3) [3,4)");
    assert_eq!(
        windows.batches[0].partition_start,
        PartitionPoint::Integer(1)
    );
    assert_eq!(windows.batches[0].partition_end, PartitionPoint::Integer(2));
    assert_eq!(
        windows.batches[1].partition_start,
        PartitionPoint::Integer(2)
    );
    assert_eq!(windows.batches[1].partition_end, PartitionPoint::Integer(3));
    assert_eq!(
        windows.batches[2].partition_start,
        PartitionPoint::Integer(3)
    );
    assert_eq!(windows.batches[2].partition_end, PartitionPoint::Integer(4));
}

#[test]
fn integer_axis_batch_size_counts_units() {
    let sql = "SELECT batch_id, id FROM events";
    let ts = make_ts("event_ts", "batch_id", Granularity::Day);
    let inc = make_inc();
    let range = make_range("1", "6");

    let windows = compute_incremental_windows(
        &ts,
        &inc,
        sql,
        &no_dep_timeseries(),
        &range,
        PartitionAxis::Integer,
        Some(2),
        false,
    )
    .expect("integer axis must not be refused");

    assert_eq!(windows.batches.len(), 3, "expected [1,3) [3,5) [5,6)");
    assert_eq!(
        windows.batches[0].partition_start,
        PartitionPoint::Integer(1)
    );
    assert_eq!(windows.batches[0].partition_end, PartitionPoint::Integer(3));
    assert_eq!(
        windows.batches[1].partition_start,
        PartitionPoint::Integer(3)
    );
    assert_eq!(windows.batches[1].partition_end, PartitionPoint::Integer(5));
    assert_eq!(
        windows.batches[2].partition_start,
        PartitionPoint::Integer(5)
    );
    assert_eq!(windows.batches[2].partition_end, PartitionPoint::Integer(6));
}

// ── validate_run_window_against_partition_grid on the integer axis ────────

#[test]
fn integer_axis_run_window_requires_positive_span_only() {
    let ts = make_ts("event_ts", "batch_id", Granularity::Day);
    let sql = "SELECT batch_id, id FROM events";

    validate_run_window_against_partition_grid(
        sql,
        &ts,
        PartitionPoint::Integer(3),
        PartitionPoint::Integer(4),
    )
    .expect("a positive-span integer window is accepted, no boundary/g_part check applies");

    let err = validate_run_window_against_partition_grid(
        sql,
        &ts,
        PartitionPoint::Integer(4),
        PartitionPoint::Integer(4),
    )
    .expect_err("a zero-span integer window must be rejected");
    assert!(
        err.contains("after"),
        "error must explain end must be after start, got: {err}"
    );
}

// ── Domain-mismatch refusals ────────────────────────────────────────────────

#[test]
fn integer_axis_refuses_date_bounds() {
    let err = PartitionPoint::parse_in_axis("2026-01-01", PartitionAxis::Integer)
        .expect_err("a calendar-shaped bound must be refused on an integer axis");
    assert!(err.contains("integer"), "got: {err}");
}

#[test]
fn calendar_axis_refuses_integer_bounds() {
    let err = PartitionPoint::parse_in_axis("7", PartitionAxis::Calendar)
        .expect_err("a bare-integer bound must be refused on a calendar axis");
    assert!(err.contains("calendar"), "got: {err}");
}

// ── Real (non-dry-run) execution resolves each batch's TimeRange in the
// model's OWN partition axis (phase 3a, `docs/outcomes/20260913-trino-
// incremental/phases/03a-plan.md`) ────────────────────────────────────────
//
// `execute_project`'s real run built the per-batch `run_range`/`scan_range`
// with a hardcoded `PartitionAxis::Calendar` (`execute/project/mod.rs`
// ~3376/~3391), so an integer-axis model's injected scan-window and
// output-clamp predicates rendered quoted (`batch_id >= '1'`) instead of
// bare. DuckDB/Spark/BigQuery coerce the quoted string implicitly; Trino
// refuses it (`Cannot apply operator: integer <= varchar(1)`). These two
// tests drive a real (non-`--dry-run`) `execute_project` call and inspect
// the batch-filtered SQL `reporter.model_compiled` receives.
mod real_run_axis {
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use smelt_backend::Backend;
    use smelt_backend_duckdb::DuckDbBackend;
    use smelt_core::config::{Config, Target};
    use smelt_core::graph::DependencyGraph;
    use smelt_core::ModelDiscovery;
    use smelt_runtime::execute::{execute_project, BackendFactory, BackendFuture};
    use smelt_runtime::reporter::RunReporter;
    use smelt_runtime::types::ExecuteRequest;
    use tokio_util::sync::CancellationToken;

    struct PlainDuckDbFactory {
        db_path: std::path::PathBuf,
    }

    impl BackendFactory for PlainDuckDbFactory {
        fn create<'a>(
            &'a self,
            _target_name: &'a str,
            target_config: &'a Target,
            _project_dir: &'a Path,
        ) -> BackendFuture<'a> {
            let path = self.db_path.clone();
            let schema = target_config.schema.clone();
            Box::pin(async move {
                let inner = DuckDbBackend::new(&path, &schema)
                    .await
                    .map_err(|e| anyhow::anyhow!("DuckDB init failed: {}", e))?;
                Ok(Box::new(inner) as Box<dyn Backend>)
            })
        }
    }

    #[derive(Default)]
    struct CapturingReporter {
        compiled_sql: Mutex<Vec<String>>,
    }

    impl RunReporter for CapturingReporter {
        fn model_compiled(&self, _run_id: &str, _model: &str, sql: &str) {
            self.compiled_sql.lock().unwrap().push(sql.to_string());
        }
    }

    fn stage_int_partition_project(project_dir: &Path, db_path: &Path) {
        std::fs::create_dir_all(project_dir.join("models")).unwrap();
        std::fs::write(
            project_dir.join("models/seed_events.sql"),
            "---\n\
             materialization: table\n\
             ---\n\
             SELECT * FROM (VALUES\n\
             \x20  (1, 1, TIMESTAMP '2026-01-01 00:00:00'),\n\
             \x20  (2, 1, TIMESTAMP '2026-01-01 06:00:00'),\n\
             \x20  (3, 2, TIMESTAMP '2026-01-02 00:00:00'),\n\
             \x20  (4, 3, TIMESTAMP '2026-01-03 00:00:00')\n\
             ) AS t(id, batch_id, event_ts)\n",
        )
        .unwrap();
        std::fs::write(
            project_dir.join("models/int_partition_mart.sql"),
            "---\n\
             materialization: table\n\
             refresh: incremental\n\
             grain: partition\n\
             timeseries:\n\
             \x20 event_time_column: event_ts\n  partition_column: batch_id\n  granularity: day\n\
             ---\n\
             SELECT CAST(batch_id AS INTEGER) AS batch_id, event_ts, id FROM smelt.seed_events\n",
        )
        .unwrap();

        let smelt_yml = format!(
            "name: int_partition_axis_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
            db = db_path.display()
        );
        std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
    }

    fn stage_calendar_partition_project(project_dir: &Path, db_path: &Path) {
        std::fs::create_dir_all(project_dir.join("models")).unwrap();
        std::fs::write(
            project_dir.join("models/seed_events.sql"),
            "---\n\
             materialization: table\n\
             ---\n\
             SELECT * FROM (VALUES\n\
             \x20  (1, DATE '2026-01-01'),\n\
             \x20  (2, DATE '2026-01-02'),\n\
             \x20  (3, DATE '2026-01-03')\n\
             ) AS t(id, event_date)\n",
        )
        .unwrap();
        std::fs::write(
            project_dir.join("models/date_partition_mart.sql"),
            "---\n\
             materialization: table\n\
             refresh: incremental\n\
             grain: partition\n\
             timeseries:\n\
             \x20 event_time_column: event_date\n  partition_column: event_date\n  granularity: day\n\
             ---\n\
             SELECT event_date, id FROM smelt.seed_events\n",
        )
        .unwrap();

        let smelt_yml = format!(
            "name: date_partition_axis_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
            db = db_path.display()
        );
        std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
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
            .map(|m| {
                db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf())
            })
            .collect();
        db.set_workspace(source_files, vec![project]);

        let graph = DependencyGraph::build(sql_models, None).expect("build graph");

        (
            Arc::new(tokio::sync::Mutex::new(db)),
            Arc::new(tokio::sync::Mutex::new(graph)),
        )
    }

    fn base_request() -> ExecuteRequest {
        ExecuteRequest {
            target: "dev".to_string(),
            select: vec![],
            exclude: vec![],
            start: None,
            end: None,
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
            invoke_external_steps: true,
            assume_external_steps_fresh: false,
        }
    }

    /// The batch-filtered SQL for an integer-axis model's real run renders
    /// bare bounds (`batch_id >= 1`), never quoted (`batch_id >= '1'`).
    #[tokio::test]
    async fn real_run_batch_sql_renders_integer_axis_bounds_bare() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().to_path_buf();
        let db_path = project_dir.join("dev.duckdb");

        stage_int_partition_project(&project_dir, &db_path);

        let config = Arc::new(Config::load(&project_dir).expect("load config"));
        let (db, graph) = build_db_and_graph(&project_dir, &config);

        let mut request = base_request();
        request.start = Some("1".to_string());
        request.end = Some("4".to_string());

        let reporter = CapturingReporter::default();

        execute_project(
            "int-axis-real-run".to_string(),
            request,
            Arc::clone(&config),
            Arc::clone(&graph),
            Arc::clone(&db),
            &project_dir,
            &PlainDuckDbFactory {
                db_path: db_path.clone(),
            },
            &reporter,
            CancellationToken::new(),
        )
        .await
        .expect("integer-axis real run must succeed");

        let compiled = reporter.compiled_sql.lock().unwrap();
        let mart_sql: Vec<&String> = compiled
            .iter()
            .filter(|sql| sql.contains("batch_id"))
            .collect();
        assert!(
            !mart_sql.is_empty(),
            "expected at least one compiled batch SQL for int_partition_mart"
        );
        for sql in &mart_sql {
            assert!(
                !sql.contains("'1'") && !sql.contains("'4'"),
                "integer-axis batch SQL must not quote its bounds, got: {sql}"
            );
        }
        let joined = mart_sql
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n---\n");
        assert!(
            joined.contains(">= 1") || joined.contains(">=1"),
            "expected a bare lower bound on batch_id, got: {joined}"
        );
    }

    /// Regression fence: the same shape on a `DATE` partition column still
    /// renders the calendar literal quoted — the fix must not invert the
    /// axis for the overwhelmingly common calendar case.
    #[tokio::test]
    async fn real_run_batch_sql_still_quotes_calendar_axis_bounds() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let project_dir = tmp.path().to_path_buf();
        let db_path = project_dir.join("dev.duckdb");

        stage_calendar_partition_project(&project_dir, &db_path);

        let config = Arc::new(Config::load(&project_dir).expect("load config"));
        let (db, graph) = build_db_and_graph(&project_dir, &config);

        let mut request = base_request();
        request.start = Some("2026-01-01".to_string());
        request.end = Some("2026-01-03".to_string());

        let reporter = CapturingReporter::default();

        execute_project(
            "calendar-axis-real-run".to_string(),
            request,
            Arc::clone(&config),
            Arc::clone(&graph),
            Arc::clone(&db),
            &project_dir,
            &PlainDuckDbFactory {
                db_path: db_path.clone(),
            },
            &reporter,
            CancellationToken::new(),
        )
        .await
        .expect("calendar-axis real run must succeed");

        let compiled = reporter.compiled_sql.lock().unwrap();
        let mart_sql: Vec<&String> = compiled
            .iter()
            .filter(|sql| sql.contains("event_date"))
            .collect();
        assert!(
            !mart_sql.is_empty(),
            "expected at least one compiled batch SQL for date_partition_mart"
        );
        let joined = mart_sql
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n---\n");
        assert!(
            joined.contains("'2026-01-01'"),
            "calendar-axis batch SQL must still quote its bounds, got: {joined}"
        );
    }
}

// ── Day-typed widening is refused fail-closed on the integer axis ─────────

#[test]
fn integer_axis_refuses_day_typed_widening() {
    let ts = make_ts("event_ts", "batch_id", Granularity::Day);
    let inc = make_inc();
    let range = make_range("1", "4");

    // A nonzero seconds/day-domain SQL-inferred lookback.
    let lag_sql = "SELECT batch_id, LAG(amount, 3) OVER (ORDER BY batch_id) as prev FROM events";
    let err = compute_incremental_windows(
        &ts,
        &inc,
        lag_sql,
        &no_dep_timeseries(),
        &range,
        PartitionAxis::Integer,
        None,
        true,
    )
    .expect_err("a nonzero SQL-inferred lookback must be refused on an integer axis");
    assert!(
        err.contains("lookback") || err.contains("lookahead"),
        "error must name the offending input, got: {err}"
    );
}
