//! Real-fixture test: `refresh: materialized_view` on a backend without native
//! incremental-view maintenance (every backend today) is a hard error — never a
//! silent fallback to `keyed` or a full-refresh table
//! (`docs/specs/materialized_view.md` §"No silent fallback").
//!
//! Exercises the actual `execute_project` pipeline (same entry point `smelt build` /
//! `smelt run` and the UI use) against a real DuckDB backend, not just the
//! `SqlCompiler::compile` unit-test path.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use smelt_backend::Backend;
use smelt_backend_duckdb::DuckDbBackend;
use smelt_core::config::{Config, Materialization, Target};
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

fn write_model(project_dir: &Path, name: &str, content: &str) {
    let path = project_dir.join("models").join(format!("{}.sql", name));
    std::fs::write(path, content).expect("write model file");
}

fn build_db_and_graph(
    project_dir: &Path,
    config: &Config,
) -> (
    Arc<tokio::sync::Mutex<smelt_db::Database>>,
    Arc<tokio::sync::Mutex<DependencyGraph>>,
) {
    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let sql_models = discovery.discover_models().expect("discover_models failed");

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

fn make_request(target: &str) -> ExecuteRequest {
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
        retry_max: None,
        retry_backoff_ms: None,
        resume: false,
        technique_overrides: vec![],
    }
}

/// `refresh: materialized_view` on DuckDB errors — never silently becomes
/// `keyed` or a full-refresh table.
#[tokio::test]
async fn test_materialized_view_hard_errors_on_duckdb() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let project_dir = tmp.path();
    std::fs::create_dir_all(project_dir.join("models")).unwrap();

    write_model(
        project_dir,
        "mv_model",
        "---\nmaterialization: table\nrefresh: materialized_view\n---\n\
         SELECT device_id, COUNT(*) AS event_count\n\
         FROM (VALUES (1), (1), (2)) AS t(device_id)\n\
         GROUP BY device_id",
    );

    let db_path = project_dir.join("test.duckdb");

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
            project: None,
            dataset: None,
            location: None,
        },
    );

    let config = Arc::new(Config {
        name: "mv_hard_error_test".to_string(),
        version: 1,
        paths: vec!["models".to_string()],
        targets,
        default_materialization: Materialization::View,
        models: HashMap::new(),
        python: None,
        target: None,
        state: Default::default(),
        maintenance: None,
        probes: Default::default(),
    });

    let (db, graph) = build_db_and_graph(project_dir, &config);

    let result = execute_project(
        "run-mv-hard-error".to_string(),
        make_request("dev"),
        Arc::clone(&config),
        Arc::clone(&graph),
        Arc::clone(&db),
        project_dir,
        &DuckDbBackendFactory {
            db_path: db_path.clone(),
        },
        &NoOpReporter,
        CancellationToken::new(),
    )
    .await;

    let err = result.expect_err(
        "refresh: materialized_view on DuckDB (no native IVM) must hard-error, \
         not silently build a table",
    );
    let message = err.to_string();
    assert!(
        message.contains("requires native incremental-view maintenance"),
        "expected the §\"No silent fallback\" hard error, got: {}",
        message
    );
    assert!(
        message.contains("use `refresh: incremental` with `grain: key`"),
        "expected the hard error to point at `refresh: incremental` + `grain: key`, got: {}",
        message
    );
}

/// `crates/smelt-backend/src/` must not contain a silent materialized-view
/// fallback surface — the compile-time hard error above is one line of
/// defense, and `Backend::create_materialized_view_as`'s own provided
/// default (an erroring `BackendError::UnsupportedFeature`, never a quiet
/// substitution) is the other. What must never appear is a fallback that
/// *succeeds* by silently routing to `create_table_as`/`create_view_as`
/// instead of refusing — that would violate `docs/specs/materialized_view.md`
/// §"No silent fallback" even though the method name itself
/// (`create_materialized_view_as`) is exactly the correct, loud-erroring
/// surface this spec calls for.
#[test]
fn no_silent_fallback_surface_in_backend_crate() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("smelt-runtime crate is two levels below repo root");
    let backend_src = repo_root.join("crates/smelt-backend/src");

    let mut offenders = vec![];
    for entry in walk_rs_files(&backend_src) {
        let content = std::fs::read_to_string(&entry).expect("read backend src file");
        if content.contains("falling back") {
            offenders.push(format!("{}: contains \"falling back\"", entry.display()));
        }
    }

    assert!(
        offenders.is_empty(),
        "smelt-backend must not expose a silent materialized-view fallback surface:\n  {}",
        offenders.join("\n  ")
    );
}

fn walk_rs_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_rs_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}
