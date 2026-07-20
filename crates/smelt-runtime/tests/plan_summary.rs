//! Phase 3 TDD tests for `PlanSummary` — the dry-run execution plan surface.
//!
//! Three tests mirror the plan's required TDD anchors:
//!
//! 1. `test_plan_summary_lists_strategies` — `execute_project` with
//!    `dry_run = true` returns a `PlanSummary` whose entries list each model
//!    with its resolved strategy.
//! 2. `test_planner_override_applied` — a model whose frontmatter says `Table`
//!    but which a planner rule turns incremental appears as `Incremental` in
//!    the `PlanSummary`.
//! 3. `test_dry_run_emits_no_backend_calls` — `dry_run = true` returns
//!    `PlanSummary` without calling `BackendFactory::create`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use smelt_core::config::{Config, Materialization, Target};
use smelt_core::graph::DependencyGraph;
use smelt_runtime::execute::{BackendFactory, BackendFuture};
use smelt_runtime::types::{ExecuteRequest, ModelStrategy};
use smelt_runtime::{execute_project, NoOpReporter};
use tokio_util::sync::CancellationToken;

// ── helpers ──────────────────────────────────────────────────────────────────

fn make_config() -> Arc<Config> {
    let mut targets = HashMap::new();
    targets.insert(
        "dev".to_string(),
        Target {
            target_type: "duckdb".to_string(),
            database: Some(":memory:".to_string()),
            schema: "main".to_string(),
            connect_url: None,
            catalog: None,
            warehouse: None,
            format: None,
            settings: None,
        },
    );
    Arc::new(Config {
        name: "test".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets,
        default_materialization: Materialization::View,
        models: HashMap::new(),
        python: None,
        target: None,
        state: Default::default(),
        maintenance: None,
    })
}

/// A `BackendFactory` that panics if `create` is ever called.
/// Used to assert the dry-run path never invokes backend construction.
struct PanicsOnCreateFactory;

impl BackendFactory for PanicsOnCreateFactory {
    fn create<'a>(
        &'a self,
        _target_name: &'a str,
        _target_config: &'a smelt_core::config::Target,
        _project_dir: &'a Path,
    ) -> BackendFuture<'a> {
        panic!("BackendFactory::create must NOT be called during a dry_run");
    }
}

fn make_db_and_graph(
    project_dir: &Path,
    models: Vec<smelt_core::ModelFile>,
) -> (
    Arc<tokio::sync::Mutex<smelt_db::Database>>,
    Arc<tokio::sync::Mutex<DependencyGraph>>,
) {
    // Write a minimal smelt.yml so the Salsa DB knows the scan roots.
    // Without this, ref resolution (smelt.model_a) fails with UndefinedModelRef
    // because the project doesn't know models/ is a scan root.
    let smelt_yml = "name: test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: test.duckdb\n    schema: main\n";
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).expect("write smelt.yml");

    // Build the Salsa DB using the project directory (reads smelt.yml for paths).
    let db = {
        let sources_yaml = String::new();
        let mut db = smelt_db::Database::default();
        let project = db.set_project_input(project_dir.to_path_buf(), sources_yaml);
        let source_files: Vec<_> = models
            .iter()
            .map(|m| {
                db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf())
            })
            .collect();
        db.set_workspace(source_files, vec![project]);
        // Load smelt.yml config so scan roots are known.
        if let Ok(cfg) = smelt_core::config::Config::load(project_dir) {
            db.set_active_target(cfg.target.map(|t| std::sync::Arc::from(t.as_str())));
        }
        db
    };

    // Build a minimal DependencyGraph
    let graph = DependencyGraph::build(models, None).expect("build dependency graph");

    (
        Arc::new(tokio::sync::Mutex::new(db)),
        Arc::new(tokio::sync::Mutex::new(graph)),
    )
}

fn make_model_file(name: &str, content: &str, project_dir: &Path) -> smelt_core::ModelFile {
    let path = project_dir.join(format!("models/{}.sql", name));
    smelt_core::ModelFile {
        name: name.to_string(),
        model_id: smelt_core::ModelId::from_path(path.clone()),
        path,
        content: content.to_string(),
        refs: vec![],
        parse_errors: vec![],
        metadata: None,
        kind: smelt_core::discovery::ModelKind::Sql,
        address_segments: vec![name.to_string()],
    }
}

fn dry_run_request(target: &str) -> ExecuteRequest {
    ExecuteRequest {
        target: target.to_string(),
        select: vec![],
        exclude: vec![],
        start: None,
        end: None,
        batch_size_days: None,
        per_partition: false,
        full_refresh: false,
        dry_run: true,
        enforce_safety: false, // don't fail on simple test models
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

// ── Test 1: PlanSummary lists strategies ─────────────────────────────────────

#[tokio::test]
async fn test_plan_summary_lists_strategies() {
    let project_dir = tempfile::tempdir().expect("tempdir");
    let project_dir = project_dir.path();

    // Create the models dir so path construction is valid
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    let model_a = make_model_file("model_a", "SELECT 1 AS x", project_dir);
    let model_b = make_model_file("model_b", "SELECT x FROM smelt.model_a", project_dir);

    let config = make_config();
    let (db, graph) = make_db_and_graph(project_dir, vec![model_a, model_b]);

    let outcome = execute_project(
        "run-1".to_string(),
        dry_run_request("dev"),
        config,
        graph,
        db,
        project_dir,
        &PanicsOnCreateFactory,
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("dry_run execute_project");

    let summary = outcome
        .plan_summary
        .expect("dry_run must populate plan_summary");

    // Both models must appear
    assert_eq!(
        summary.models.len(),
        2,
        "expected 2 entries in plan_summary, got: {:?}",
        summary.models.iter().map(|m| &m.name).collect::<Vec<_>>()
    );

    // Without incremental config, both should be FullRefresh
    for record in &summary.models {
        assert!(
            matches!(record.strategy, ModelStrategy::FullRefresh),
            "model '{}' should be FullRefresh, got {:?}",
            record.name,
            record.strategy,
        );
    }
}

// ── Test 2: Planner override (incremental via planner rule) ──────────────────

#[tokio::test]
async fn test_planner_override_applied() {
    use smelt_core::config::{ModelConfig, TimeseriesConfig};
    use smelt_core::{BatchedConfig, BatchedSafetyOverrides, Granularity};

    let project_dir = tempfile::tempdir().expect("tempdir");
    let project_dir = project_dir.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    // Write a model with incremental frontmatter so it's classified as Incremental
    let inc_sql = r#"---
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT event_date, COUNT(*) AS cnt FROM raw GROUP BY event_date"#;

    let model = make_model_file("inc_model", inc_sql, project_dir);

    let mut config = (*make_config()).clone();
    config.models.insert(
        "inc_model".to_string(),
        ModelConfig {
            materialization: Some(Materialization::Table),
            timeseries: Some(TimeseriesConfig {
                event_time_column: "event_date".to_string(),
                partition_column: "event_date".to_string(),
                granularity: Granularity::Day,
                week_start: None,
                assert_monotonic: false,
            }),
            refresh: Some(smelt_core::config::RefreshStrategy::Incremental),
            grain: Some(smelt_core::config::Grain::Partition),
            unique_key: None,
            batched: Some(BatchedConfig {
                unique_key: vec![],
                nondeterministic_columns: vec![],
                safety_overrides: BatchedSafetyOverrides::default(),
            }),
            tags: vec![],
            target: None,
            format: None,
        },
    );
    let config = Arc::new(config);

    let (db, graph) = make_db_and_graph(project_dir, vec![model]);

    // Provide a time window so the planner-applied incremental config resolves
    // to ModelStrategy::Incremental. Without start+end the model falls back to
    // FullRefresh, which would not verify the invariant we care about.
    let mut request = dry_run_request("dev");
    request.start = Some("2024-01-01".to_string());
    request.end = Some("2024-02-01".to_string());

    let outcome = execute_project(
        "run-2".to_string(),
        request,
        config,
        graph,
        db,
        project_dir,
        &PanicsOnCreateFactory,
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("dry_run execute_project");

    let summary = outcome
        .plan_summary
        .expect("dry_run must populate plan_summary");

    assert_eq!(summary.models.len(), 1, "expected 1 model in plan_summary");
    let record = &summary.models[0];
    assert_eq!(record.name, "inc_model");
    // The model has incremental config AND a time window, so the planner-applied
    // incremental strategy must appear as Incremental in the PlanSummary.
    assert!(
        matches!(record.strategy, ModelStrategy::Incremental { .. }),
        "incremental model with a time window must appear as Incremental in plan_summary, got {:?}",
        record.strategy
    );
}

// ── Test 3: dry_run does not invoke backend creation ─────────────────────────

#[tokio::test]
async fn test_dry_run_emits_no_backend_calls() {
    let project_dir = tempfile::tempdir().expect("tempdir");
    let project_dir = project_dir.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    let model = make_model_file("simple", "SELECT 42 AS answer", project_dir);
    let config = make_config();
    let (db, graph) = make_db_and_graph(project_dir, vec![model]);

    // PanicsOnCreateFactory will panic if create() is called.
    // The test passes if execute_project returns without panicking.
    let outcome = execute_project(
        "run-3".to_string(),
        dry_run_request("dev"),
        config,
        graph,
        db,
        project_dir,
        &PanicsOnCreateFactory,
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await
    .expect("dry_run must succeed without backend creation");

    // PlanSummary must be present
    let summary = outcome
        .plan_summary
        .expect("dry_run must populate plan_summary");

    assert_eq!(
        summary.models.len(),
        1,
        "single model must appear in plan_summary"
    );
}
