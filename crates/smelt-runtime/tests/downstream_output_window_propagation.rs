//! The derived output window propagates within a run (`docs/specs/
//! model_transforms.md` §Semantics "The derived output window propagates
//! within a run."): a model's own run window is the union of the requested
//! run window and the derived output window of every upstream maintained
//! model selected in the same invocation. Without this, a Form-A downstream
//! that reads a Form-B upstream verbatim never revisits the partitions the
//! upstream rebases on its own skew, and freezes at first-write time.

use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::NaiveDate;

use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::{Granularity, ModelDiscovery};
use smelt_logical::maintenance::emit::StatementGroup;
use smelt_runtime::execute::{execute_project, BackendFactory, BackendFuture};
use smelt_runtime::reporter::{ChunkInfo, RunReporter};
use smelt_runtime::types::ExecuteRequest;
use smelt_runtime::windowing::{
    widen_run_window_for_upstream_outputs, IncrementalBatch, IncrementalWindows, PartitionPoint,
};
use tokio_util::sync::CancellationToken;

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn dp(s: &str) -> PartitionPoint {
    PartitionPoint::Date(date(s))
}

fn batch(ps: &str, pe: &str) -> IncrementalBatch {
    IncrementalBatch {
        partition_start: dp(ps),
        partition_end: dp(pe),
        filter_start: dp(ps),
        filter_end: dp(pe),
        scan_start: dp(ps),
        scan_end: dp(pe),
    }
}

fn zero_effective_window() -> smelt_planner::EffectiveWindow {
    smelt_planner::EffectiveWindow {
        lookback_days: 0,
        lookahead_days: 0,
        is_unbounded: false,
        explanation: String::new(),
    }
}

// --- Test 1: `IncrementalWindows::output_window()` ---

#[test]
fn output_window_reports_the_batch_tiling_envelope() {
    let windows = IncrementalWindows {
        batches: vec![
            batch("2024-01-01", "2024-01-03"),
            batch("2024-01-03", "2024-01-05"),
        ],
        effective_window: zero_effective_window(),
        wide_batch_warning: None,
        skew: smelt_logical::analysis::source_bounds::Skew::ZERO,
    };
    assert_eq!(
        windows.output_window(),
        Some((dp("2024-01-01"), dp("2024-01-05")))
    );
}

#[test]
fn output_window_is_none_for_an_empty_batch_list() {
    let windows = IncrementalWindows {
        batches: vec![],
        effective_window: zero_effective_window(),
        wide_batch_warning: None,
        skew: smelt_logical::analysis::source_bounds::Skew::ZERO,
    };
    assert_eq!(windows.output_window(), None);
}

// --- Tests 2-5: the pure widening helper ---

#[test]
fn a_form_b_upstream_widens_a_form_a_downstream_run_window() {
    let requested = (dp("2024-06-10"), dp("2024-06-11"));
    let upstream_output = (dp("2024-06-09"), dp("2024-06-12"));
    let widened =
        widen_run_window_for_upstream_outputs(requested, &[upstream_output], &Granularity::Day);
    assert_eq!(widened, upstream_output);
}

#[test]
fn an_upstream_window_inside_the_run_window_widens_nothing() {
    let requested = (dp("2024-06-10"), dp("2024-06-11"));
    // Zero-skew upstream: its output window equals the requested window,
    // already inside — the union is a no-op.
    let upstream_output = (dp("2024-06-10"), dp("2024-06-11"));
    let widened =
        widen_run_window_for_upstream_outputs(requested, &[upstream_output], &Granularity::Day);
    assert_eq!(widened, requested);
}

#[test]
fn propagated_widening_aligns_outward_to_the_downstream_granularity() {
    // Downstream is month-grained; a day-grained upstream output window pokes
    // a few days past the month boundary on both sides.
    let requested = (dp("2024-03-01"), dp("2024-04-01"));
    let upstream_output = (dp("2024-02-28"), dp("2024-04-02"));
    let widened =
        widen_run_window_for_upstream_outputs(requested, &[upstream_output], &Granularity::Month);
    assert_eq!(widened, (dp("2024-02-01"), dp("2024-05-01")));
}

#[test]
fn a_mismatched_partition_axis_propagates_nothing() {
    let requested = (PartitionPoint::Integer(10), PartitionPoint::Integer(11));
    let upstream_output = (dp("2024-06-09"), dp("2024-06-12"));
    let widened =
        widen_run_window_for_upstream_outputs(requested, &[upstream_output], &Granularity::Day);
    assert_eq!(widened, requested);
}

// --- Tests 6-7: end-to-end through execute_project ---

/// One recorded `maintenance_statements` callback: model name, the chunk
/// window (index, total, start, end).
type RecordedCall = (
    String,
    Option<(usize, usize, String, String)>,
    StatementGroup,
);

#[derive(Default)]
struct RecordingReporter {
    calls: Mutex<Vec<RecordedCall>>,
}

impl RunReporter for RecordingReporter {
    fn maintenance_statements(
        &self,
        _run_id: &str,
        model: &str,
        chunk: Option<&ChunkInfo>,
        group: &StatementGroup,
    ) {
        self.calls.lock().unwrap().push((
            model.to_string(),
            chunk.map(|c| (c.index, c.total, c.start.clone(), c.end.clone())),
            group.clone(),
        ));
    }
}

/// A `BackendFactory` that panics if `create` is ever called — a dry-run
/// must never construct a backend.
struct PanicBackendFactory;

impl BackendFactory for PanicBackendFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        _target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        Box::pin(async move {
            panic!("dry-run must not create a backend");
        })
    }
}

fn write_nested_model(project_dir: &Path, addr: &str, content: &str) {
    let mut segs: Vec<&str> = addr.split('.').collect();
    let leaf = segs.pop().expect("address has at least one segment");
    let dir = segs
        .iter()
        .fold(project_dir.join("models"), |acc, s| acc.join(s));
    std::fs::create_dir_all(&dir).expect("mkdir nested model dir");
    std::fs::write(dir.join(format!("{leaf}.sql")), content).expect("write nested model file");
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
    db.set_active_target(config.target.clone().map(|t| Arc::from(t.as_str())));

    let graph = DependencyGraph::build(sql_models, None).expect("build graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

fn dry_run_request(select: Vec<String>, start: &str, end: &str) -> ExecuteRequest {
    ExecuteRequest {
        target: "dev".to_string(),
        select,
        exclude: vec![],
        start: Some(start.to_string()),
        end: Some(end.to_string()),
        batch_size_days: None,
        per_partition: false,
        full_refresh: false,
        rebuild: false,
        dry_run: true,
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

/// A Form-B upstream declaring a symmetric one-day-either-side relation
/// (`event_date BETWEEN d - INTERVAL '1 day' AND d + INTERVAL '1 day'`) —
/// `Skew { before: 1 day, after: 1 day }`. A [`D, D+1`) run therefore
/// derives an output window of `[D-1, D+2)`.
const UPSTREAM_SQL: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: partition\n\
     timeseries:\n\
     \x20\x20partition_column: d\n\
     \x20\x20event_time_column: driving\n\
     \x20\x20granularity: day\n\
     ---\n\
     SELECT d, driving, SUM(amount) AS total FROM \
     (VALUES (DATE '2024-01-01', DATE '2024-01-01', 10), \
     (DATE '2024-01-02', DATE '2024-01-02', 20), \
     (DATE '2024-01-03', DATE '2024-01-03', 30), \
     (DATE '2024-01-04', DATE '2024-01-04', 40)) AS t(d, driving, amount) \
     WHERE driving BETWEEN d - INTERVAL '1 day' AND d + INTERVAL '1 day' \
     GROUP BY d, driving";

/// Form A over `upstream`: reads its partition column verbatim, no skew of
/// its own.
const DOWNSTREAM_SQL: &str = "---\n\
     materialization: table\n\
     refresh: incremental\n\
     grain: partition\n\
     timeseries:\n\
     \x20\x20partition_column: d\n\
     \x20\x20event_time_column: d\n\
     \x20\x20granularity: day\n\
     ---\n\
     SELECT d, total FROM smelt.upstream";

fn write_project(project_dir: &Path, db_path: &Path) -> Arc<Config> {
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    write_nested_model(project_dir, "upstream", UPSTREAM_SQL);
    write_nested_model(project_dir, "downstream", DOWNSTREAM_SQL);
    let smelt_yml = format!(
        "name: output_window_propagation_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\ntarget: dev\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), &smelt_yml).unwrap();
    Arc::new(Config::load(project_dir).expect("load config"))
}

/// A Form-A downstream reading a Form-B upstream verbatim runs over the
/// upstream's rebased output window, not the bare requested window.
#[tokio::test]
async fn a_form_a_downstream_runs_over_its_form_b_upstreams_rebased_window() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    let db_path = project_dir.join("run.duckdb");
    let config = write_project(project_dir, &db_path);

    let (db, graph) = build_db_and_graph(project_dir, &config);
    let reporter = RecordingReporter::default();

    execute_project(
        "output-window-propagation".to_string(),
        dry_run_request(vec![], "2024-01-02", "2024-01-03"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &PanicBackendFactory,
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect("dry-run execute_project");

    let calls = reporter.calls.lock().unwrap();
    let downstream_chunks: Vec<_> = calls.iter().filter(|(m, _, _)| m == "downstream").collect();
    assert_eq!(
        downstream_chunks.len(),
        1,
        "expected one chunk for the downstream: {:?}",
        calls
    );
    let (_, chunk, _) = downstream_chunks[0];
    let (_, _, start, end) = chunk.as_ref().expect("chunk info present");
    assert_eq!(
        (start.as_str(), end.as_str()),
        ("2024-01-01", "2024-01-04"),
        "downstream's run window must widen to the upstream's derived output \
         window [D-1, D+2)"
    );
}

/// When the upstream isn't selected in the invocation, it never contributes
/// an output window — the downstream keeps the requested window verbatim.
#[tokio::test]
async fn an_unselected_upstream_does_not_widen_its_downstream() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    let db_path = project_dir.join("run.duckdb");
    let config = write_project(project_dir, &db_path);

    let (db, graph) = build_db_and_graph(project_dir, &config);
    let reporter = RecordingReporter::default();

    execute_project(
        "output-window-propagation-unselected".to_string(),
        dry_run_request(vec!["downstream".to_string()], "2024-01-02", "2024-01-03"),
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &PanicBackendFactory,
        &reporter,
        CancellationToken::new(),
    )
    .await
    .expect("dry-run execute_project");

    let calls = reporter.calls.lock().unwrap();
    assert!(
        calls.iter().all(|(m, _, _)| m != "upstream"),
        "upstream must not have run: {:?}",
        calls
    );
    let downstream_chunks: Vec<_> = calls.iter().filter(|(m, _, _)| m == "downstream").collect();
    assert_eq!(downstream_chunks.len(), 1);
    let (_, chunk, _) = downstream_chunks[0];
    let (_, _, start, end) = chunk.as_ref().expect("chunk info present");
    assert_eq!(
        (start.as_str(), end.as_str()),
        ("2024-01-02", "2024-01-03"),
        "with the upstream unselected, the downstream's run window is the \
         requested window verbatim"
    );
}
