#![cfg(feature = "duckdb")]
//! Phase 8 TDD tests for the run-report artifact, `--log-format`, and the
//! end-of-run failure summary
//! (`docs/plans/20260719-prod-w2-operability.md` §"Phase 8: Run-report
//! artifact, structured logs, failure summary"; `docs/specs/run_state.md`
//! §"Run report").

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use smelt_core::config::{
    Config, Materialization as CfgMaterialization, ModelConfig, StateMode, Target,
};
use smelt_core::graph::DependencyGraph;
use smelt_core::ModelDiscovery;
use smelt_runtime::execute_project;
use smelt_runtime::reporter::NoOpReporter;
use smelt_runtime::types::ExecuteRequest;
use smelt_state::file_store::FileStore;
use smelt_state::RunOutcomeKind;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

// ── Fixture helpers (mirrors tests/resume.rs) ───────────────────────────────

fn write_model(project_dir: &Path, name: &str, content: &str) {
    let path = project_dir.join("models").join(format!("{}.sql", name));
    std::fs::write(path, content).expect("write model file");
}

fn write_config(project_dir: &Path, db_path: &Path) {
    let smelt_yml = format!(
        "name: run_report_test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: {db}\n    schema: main\ndefault_materialization: table\n",
        db = db_path.display()
    );
    std::fs::write(project_dir.join("smelt.yml"), smelt_yml).unwrap();
}

fn make_config(db_path: &Path) -> Arc<Config> {
    let mut targets = HashMap::new();
    targets.insert(
        "dev".to_string(),
        Target {
            target_type: "duckdb".to_string(),
            database: Some(db_path.to_string_lossy().into_owned()),
            schema: "main".to_string(),
            connect_url: None,
            catalog: None,
            warehouse: None,
            format: None,
            settings: None,
        },
    );
    Arc::new(Config {
        name: "run_report_test".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets,
        default_materialization: CfgMaterialization::Table,
        models: HashMap::<String, ModelConfig>::new(),
        python: None,
        target: None,
        // The run report is a `StateFamily::Report` observability write —
        // this suite reads it back, so it needs a posture that writes it
        // (`docs/specs/state.md` §"`state.mode` and what each posture
        // provides"). The default `stateless` posture writes none.
        state: smelt_core::config::StateConfig {
            mode: StateMode::Environments,
        },
        maintenance: None,
        probes: Default::default(),
    })
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

fn base_request(target: &str) -> ExecuteRequest {
    ExecuteRequest {
        target: target.to_string(),
        select: vec![],
        exclude: vec![],
        start: None,
        end: None,
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
        retry_max: Some(0),
        retry_backoff_ms: Some(1),
        resume: false,
        technique_overrides: vec![],
    }
}

/// A report is written on both a clean run and a run where independent
/// models fail in the same wave — every failing model gets its own entry
/// with its own error text, not just the first (`docs/specs/run_state.md`
/// §"Run manifest": abort semantics "let the in-flight wave finish, record
/// every failure, then abort").
#[tokio::test]
async fn report_written_on_success_and_on_failure() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();
    write_model(project_dir, "up", "SELECT 1 AS id, 'a' AS val");

    let db_path = project_dir.join("db.duckdb");
    write_config(project_dir, &db_path);
    let config = make_config(&db_path);
    let reporter = NoOpReporter;

    // ── Run 1: succeeds. ─────────────────────────────────────────────────
    {
        let (db, graph) = build_db_and_graph(project_dir, &config);
        execute_project(
            "run-success".to_string(),
            base_request("dev"),
            Arc::clone(&config),
            graph,
            db,
            project_dir,
            &smelt_backend_duckdb_factory(),
            &reporter,
            CancellationToken::new(),
        )
        .await
        .expect("run 1 must succeed");
    }

    let file_store = FileStore::new(project_dir, "dev", StateMode::Environments);
    let report = file_store
        .load_report("run-success")
        .expect("load_report must not error")
        .expect("report must exist for a successful run");
    assert_eq!(report.run_id, "run-success");
    assert!(
        report.completed_at.is_some(),
        "a successful run's report has a completed_at"
    );
    assert_eq!(report.outcome_counts.success, 1);
    assert_eq!(report.outcome_counts.failed, 0);
    assert!(report.failures.is_empty());

    // ── Run 2: two independent models fail in the same wave. ─────────────
    write_model(
        project_dir,
        "bad_a",
        "SELECT CAST('not_a_number' AS INT) AS id",
    );
    write_model(
        project_dir,
        "bad_b",
        "SELECT CAST('also_not_a_number' AS INT) AS id",
    );

    let (db, graph) = build_db_and_graph(project_dir, &config);
    let mut failing_request = base_request("dev");
    failing_request.select = vec!["bad_a".to_string(), "bad_b".to_string()];
    let result = execute_project(
        "run-failure".to_string(),
        failing_request,
        Arc::clone(&config),
        graph,
        db,
        project_dir,
        &smelt_backend_duckdb_factory(),
        &reporter,
        CancellationToken::new(),
    )
    .await;
    assert!(result.is_err(), "both bad_a and bad_b must fail");

    let failed_manifest = file_store
        .load_run("run-failure")
        .expect("load_run must not error")
        .expect("failed run's manifest must be persisted");
    assert_eq!(
        failed_manifest.models["bad_a"].outcome,
        RunOutcomeKind::Failed
    );
    assert_eq!(
        failed_manifest.models["bad_b"].outcome,
        RunOutcomeKind::Failed
    );
    assert!(
        failed_manifest.models["bad_a"].error.is_some(),
        "the manifest entry itself must carry error text, not just the report"
    );
    assert!(failed_manifest.models["bad_b"].error.is_some());

    let failed_report = file_store
        .load_report("run-failure")
        .expect("load_report must not error")
        .expect("report must exist for a failed run");
    assert!(
        failed_report.completed_at.is_none(),
        "an aborted run's report is derived from an incomplete manifest"
    );
    assert_eq!(
        failed_report.outcome_counts.failed, 2,
        "both bad_a and bad_b must be counted as failed, not just the first"
    );
    let failed_models: Vec<&str> = failed_report
        .failures
        .iter()
        .map(|f| f.model.as_str())
        .collect();
    assert!(
        failed_models.contains(&"bad_a") && failed_models.contains(&"bad_b"),
        "the report must name BOTH failed models, not silently downgrade the second \
         to skipped: {failed_models:?}"
    );
    for failure in &failed_report.failures {
        assert!(
            !failure.error.is_empty(),
            "each failure must carry its own error text"
        );
    }
}

fn smelt_backend_duckdb_factory() -> smelt_cli::backend_factory::CliBackendFactory {
    smelt_cli::backend_factory::CliBackendFactory {
        database_override: None,
    }
}

// ── CLI-binary-level tests ──────────────────────────────────────────────────

fn smelt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_smelt"))
}

fn scaffold(tmp: &TempDir) -> PathBuf {
    let project_dir = tmp.path().join("proj");
    let init_out = Command::new(smelt_bin())
        .arg("init")
        .arg(&project_dir)
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt init`: {e}"));
    assert!(
        init_out.status.success(),
        "smelt init should succeed.\nstderr: {}",
        String::from_utf8_lossy(&init_out.stderr)
    );
    // `smelt init`'s template project defaults to `state.mode: stateless`
    // (`docs/specs/state.md` §"`state.mode` and what each posture
    // provides"), which writes no run report — this suite's callers
    // (`failure_summary_lists_all_failed_models`) read the report back, so
    // opt the scaffolded project into `intervals`.
    let smelt_yml_path = project_dir.join("smelt.yml");
    let mut smelt_yml =
        std::fs::read_to_string(&smelt_yml_path).expect("scaffolded smelt.yml must exist");
    smelt_yml.push_str("state:\n  mode: intervals\n");
    std::fs::write(&smelt_yml_path, smelt_yml).unwrap();
    project_dir
}

/// `--log-format json` makes every tracing-emitted line on stdout a
/// parseable JSON object, for orchestrator/log-aggregator consumption.
#[test]
fn log_format_json_emits_parseable_lines() {
    let tmp = TempDir::new().unwrap();
    let project_dir = scaffold(&tmp);

    let out = Command::new(smelt_bin())
        .arg("build")
        .arg("--log-format")
        .arg("json")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env("RUST_LOG", "info")
        .env_remove("NO_COLOR")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt build --log-format json`: {e}"));
    assert!(
        out.status.success(),
        "smelt build should succeed.\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json_lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(
        !json_lines.is_empty(),
        "expected at least one tracing line on stdout:\n{stdout}"
    );
    for line in json_lines {
        serde_json::from_str::<serde_json::Value>(line)
            .unwrap_or_else(|e| panic!("line is not valid JSON ({e}): {line}"));
    }
}

/// A run where more than one selected model fails prints one summary block
/// naming each failed model with its first error line — not just the first
/// failure, and not scattered across separate un-batched lines.
#[test]
fn failure_summary_lists_all_failed_models() {
    let tmp = TempDir::new().unwrap();
    let project_dir = scaffold(&tmp);

    // `materialization: table` forces an eager `CREATE TABLE AS`, so the
    // cast error surfaces at execution time — the default `view`
    // materialization would only error lazily on first SELECT, never
    // during `smelt run`.
    std::fs::write(
        project_dir.join("models").join("bad_a.sql"),
        "---\nmaterialization: table\n---\nSELECT CAST('not_a_number' AS INT) AS id\n",
    )
    .unwrap();
    std::fs::write(
        project_dir.join("models").join("bad_b.sql"),
        "---\nmaterialization: table\n---\nSELECT CAST('also_not_a_number' AS INT) AS id\n",
    )
    .unwrap();

    let out = Command::new(smelt_bin())
        .arg("run")
        .arg("--select")
        .arg("bad_a")
        .arg("--select")
        .arg("bad_b")
        .args(["--project-dir", project_dir.to_str().unwrap()])
        .env_remove("RUST_LOG")
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn `smelt run`: {e}"));

    assert!(
        !out.status.success(),
        "run should fail — both bad_a and bad_b error at execution time"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bad_a") && stderr.contains("bad_b"),
        "failure summary must name both failed models:\n{stderr}"
    );
    assert!(
        stderr.contains("2 model(s) failed"),
        "failure summary must state the failure count:\n{stderr}"
    );
}
