//! TDD tests for Phase 4 of `docs/plans/20260725-ui-model-diagnostics.md`
//! ("`smelt-ui` — REST endpoint"). Oracle: `docs/specs/ui_model_diagnostics.md`
//! §Surface "UI REST API"; §Semantics "Thin-consumer boundary".
//!
//! `GET /api/models/:name/diagnostics` is a thin adapter over
//! `smelt_runtime::diagnostics::build_model_diagnostics` — it derives
//! nothing itself. These tests assert:
//! - the route returns the full `ModelDiagnostics` payload for a real
//!   fixture model, and that payload agrees field-for-field with a second,
//!   independently-assembled call into the same shared builder (mirroring
//!   `smelt-cli`'s own assembly in `commands/explain.rs`, but loaded via a
//!   separate discovery path — `ModelDiscovery` + a hand-built `Database`
//!   rather than `load_workspace`/`ingest_loaded_workspace`) — proving the
//!   HTTP handler ships the shared builder's output verbatim rather than
//!   reshaping or dropping fields (`docs/specs/ui_model_diagnostics.md`
//!   §Semantics "Thin-consumer boundary"; §Design "Why a shared
//!   smelt-runtime builder"). `smelt-cli`'s own `--json` output is covered
//!   by `crates/smelt-cli/tests/explain.rs`'s
//!   `json_includes_full_preview_array_and_properties`; spawning the
//!   `smelt` binary from this crate's tests isn't possible — Cargo only
//!   sets `CARGO_BIN_EXE_<name>` for a package's own binary targets, never
//!   a dependency's;
//! - unknown models 404, matching the existing `/api/models/:name`
//!   convention exactly.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use smelt_core::config::Target;
use smelt_core::discovery::{ModelDiscovery, ModelFile};
use smelt_core::graph::DependencyGraph;
use smelt_core::workspace::load_workspace;
use smelt_db::{workspace_ingest::ingest_loaded_workspace, Database};
use smelt_planner::BoundContext;
use smelt_ui::server::AppState;
use std::collections::HashMap;
use std::path::{Path as StdPath, PathBuf};
use std::sync::Arc;
use tokio::sync::broadcast;
use tower::ServiceExt;

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples")
}

fn timeseries_project_dir() -> PathBuf {
    examples_dir().join("timeseries")
}

/// Build a test `AppState` from a real on-disk project root — the same
/// loading path `state_from_project` in `tests/api_canonical_paths.rs`
/// uses, duplicated here (test-only helper, no shared test-util crate in
/// this workspace) so this file stays self-contained.
fn state_from_project(project_root: PathBuf) -> Arc<AppState> {
    let loaded = load_workspace(&project_root);
    let mut db = Database::default();
    let ingested = ingest_loaded_workspace(&mut db, &loaded);
    db.set_workspace(ingested.source_files, vec![ingested.project]);

    let config = loaded.config.clone();
    let sources = smelt_core::SourcesConfig::load(&project_root).ok();

    let models: Vec<ModelFile> = loaded
        .sql_files
        .into_iter()
        .filter(|m| matches!(m.kind, smelt_core::ModelKind::Sql))
        .filter(|m| !m.is_assertion())
        .filter(|m| {
            let rel = m.path.strip_prefix(&project_root).unwrap_or(&m.path);
            let first = rel
                .components()
                .next()
                .map(|c| c.as_os_str().to_string_lossy().into_owned());
            config
                .paths
                .iter()
                .any(|p| Some(p.as_str()) == first.as_deref())
        })
        .collect();

    let graph = DependencyGraph::build(models, sources.as_ref()).unwrap();

    let (change_tx, _) = broadcast::channel(16);
    let (run_event_tx, _) = broadcast::channel(16);

    let run_manager = Arc::new(smelt_ui::RunManager::new(
        run_event_tx.clone(),
        project_root.clone(),
    ));

    let mut targets = HashMap::new();
    targets.insert(
        "dev".to_string(),
        Target {
            target_type: "duckdb".to_string(),
            database: Some("target/dev.duckdb".to_string()),
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
    let mut config = config;
    if config.targets.is_empty() {
        config.targets = targets;
    }

    Arc::new(AppState {
        db: Arc::new(tokio::sync::Mutex::new(db)),
        config: Arc::new(config),
        sources: Arc::new(sources),
        graph: Arc::new(tokio::sync::Mutex::new(graph)),
        project_dir: project_root,
        change_tx,
        run_manager,
        run_event_tx,
    })
}

fn test_app(state: Arc<AppState>) -> Router {
    use axum::extract::{Path, State};
    use axum::Json;
    use smelt_ui::build;

    Router::new()
        .route(
            "/api/models/{name}",
            get(
                |State(state): State<Arc<AppState>>, Path(name): Path<String>| async move {
                    let graph = state.graph.lock().await;
                    let db = state.db.lock().await;
                    let details = build::build_model_details(&graph, &state.config, &db);
                    details
                        .get(&name)
                        .cloned()
                        .map(Json)
                        .ok_or(StatusCode::NOT_FOUND)
                },
            ),
        )
        .route(
            "/api/models/{name}/diagnostics",
            get(
                |State(state): State<Arc<AppState>>, Path(name): Path<String>| async move {
                    let graph = state.graph.lock().await;
                    let db = state.db.lock().await;
                    match build::build_model_diagnostics_response(
                        &graph,
                        &state.config,
                        &db,
                        &state.project_dir,
                        &name,
                    ) {
                        Ok(Some(diagnostics)) => Ok(Json(diagnostics)),
                        Ok(None) => Err(StatusCode::NOT_FOUND),
                        Err(e) => {
                            eprintln!("diagnostics build failed for `{name}`: {e:?}");
                            Err(StatusCode::INTERNAL_SERVER_ERROR)
                        }
                    }
                },
            ),
        )
        .with_state(state)
}

/// Independently assemble a model's `ModelDiagnostics` — mirroring
/// `smelt-cli`'s own argument assembly in
/// `commands/explain.rs::explain_maintenance_plan` field-for-field, but
/// loaded through a completely separate path (`ModelDiscovery` + a
/// hand-built `Database`, rather than `state_from_project`'s
/// `load_workspace`/`ingest_loaded_workspace`) — then calls the same shared
/// `smelt_runtime::diagnostics::build_model_diagnostics` the HTTP endpoint
/// calls. Comparing this against the endpoint's JSON response proves the
/// handler ships the builder's output verbatim.
fn assemble_diagnostics_independently(
    project_dir: &StdPath,
    model_name: &str,
) -> serde_json::Value {
    let config = smelt_core::config::Config::load(project_dir).expect("load smelt.yml");
    let sources = smelt_core::SourcesConfig::load(project_dir).ok();

    let discovery = ModelDiscovery::new(project_dir.to_path_buf(), config.paths.clone());
    let models: Vec<ModelFile> = discovery
        .discover_models()
        .expect("discover models")
        .into_iter()
        .filter(|m| !m.is_assertion())
        .collect();

    let mut db = Database::default();
    let project = db.set_project_input(project_dir.to_path_buf(), String::new());
    let mut source_files = Vec::new();
    for m in &models {
        let sf = db.set_source_file(m.path.clone(), m.content.clone(), project_dir.to_path_buf());
        source_files.push(sf);
    }
    db.set_workspace(source_files, vec![project]);
    let ws = smelt_db::Workspace::try_get(&db).expect("workspace initialized");

    let model = models
        .iter()
        .find(|m| m.canonical_path() == model_name)
        .unwrap_or_else(|| panic!("model `{model_name}` not found"));
    let file = db.source_file(&model.path).expect("model file registered");

    let (plan_cells, column_groups): (
        Vec<smelt_logical::maintenance::PlanCell>,
        Vec<smelt_logical::maintenance::ColumnGroup>,
    ) = smelt_db::maintenance_plan_report(&db, ws, file)
        .map(|result| (result.plan.cells, result.column_groups))
        .unwrap_or_default();

    let graph = DependencyGraph::build(models.clone(), sources.as_ref()).expect("build graph");
    let upstream = graph.get_upstream(model_name);
    let source_infos = smelt_core::discover_source_infos(project_dir, &config.paths);

    let mut bound_ctx = BoundContext::new();
    for dep_name in &upstream {
        if let Ok(dep_model) = graph.get_model(dep_name) {
            let dep_meta = dep_model.metadata.as_deref();
            let ts = config
                .get_timeseries_with_metadata(dep_name, dep_meta)
                .cloned()
                .or_else(|| dep_meta.and_then(|m| m.timeseries.clone()));
            if let Some(ts) = ts {
                bound_ctx.add_source(dep_name, &ts.partition_column);
            }
        }
    }

    let default_target = config.targets.keys().next().cloned().unwrap_or_default();
    let target = config.get_target(model_name, model.metadata.as_deref(), &default_target);
    let schema = config
        .targets
        .get(&target)
        .map(|t| t.schema.clone())
        .unwrap_or_else(|| "main".to_string());
    let dialect = config
        .targets
        .get(&target)
        .and_then(|t| t.backend_type().ok())
        .map(|bt| match bt {
            smelt_core::config::BackendType::DuckDB => smelt_backend::SqlDialect::DuckDB,
            smelt_core::config::BackendType::Spark => smelt_backend::SqlDialect::SparkSQL,
            smelt_core::config::BackendType::BigQuery => smelt_backend::SqlDialect::BigQuery,
        })
        .map(smelt_backend::maintenance_dialect)
        .unwrap_or(smelt_logical::maintenance::emit::MaintenanceDialect::DuckDb);

    let mut registry = smelt_runtime::CompilerRegistry::new(&config, &config.targets);
    let fn_bodies = smelt_runtime::build_fn_body_map(&db, ws);
    registry.set_function_bodies_all(fn_bodies);
    if let Ok(upstream_schemas) =
        smelt_runtime::UpstreamSchemas::from_database(&db, project_dir, &models)
    {
        registry.set_upstream_schemas_all(std::sync::Arc::new(upstream_schemas));
    }

    let ephemeral_models: Vec<(String, String)> = models
        .iter()
        .filter(|m| {
            config.get_materialization_with_metadata(&m.canonical_path(), m.metadata.as_deref())
                == smelt_core::config::Materialization::Ephemeral
        })
        .map(|m| (m.db_name_owned(), m.content.clone()))
        .collect();
    let resolver = registry
        .get(&target)
        .build_ephemeral_resolver(&ephemeral_models, &schema)
        .expect("no ephemeral model here declares an unsupported construct");

    let unique_key: Vec<String> = config
        .get_incremental_with_metadata(model_name, model.metadata.as_deref())
        .map(|b| b.unique_key)
        .unwrap_or_default();

    let source_timeseries = smelt_runtime::build_source_timeseries_map(&graph, &source_infos);

    let diagnostics = smelt_runtime::diagnostics::build_model_diagnostics(
        model,
        &models,
        &upstream,
        &source_infos,
        &bound_ctx,
        &plan_cells,
        &schema,
        &target,
        &registry,
        &resolver,
        dialect,
        &source_timeseries,
        &unique_key,
        &column_groups,
    )
    .expect("build diagnostics");

    serde_json::to_value(diagnostics).expect("serialize diagnostics")
}

/// `GET /api/models/daily_events/diagnostics` must return the full
/// `ModelDiagnostics` payload, and its overlapping fields (relation
/// contract, property set, and each cell's group/trigger/corner/admitted
/// technique/technique-preview set) must agree with `smelt explain
/// daily_events --show-sql --json`'s own fields for the same model — both
/// consumers read the same shared `smelt-runtime::diagnostics` builder
/// output verbatim (`docs/specs/ui_model_diagnostics.md` §Semantics
/// "Thin-consumer boundary").
#[tokio::test]
async fn diagnostics_endpoint_returns_full_payload() {
    let project_dir = timeseries_project_dir();
    let state = state_from_project(project_dir.clone());
    let app = test_app(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/models/daily_events/diagnostics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let endpoint_json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // The response deserializes into the expected shape.
    assert_eq!(endpoint_json["model"], "daily_events");
    assert!(
        endpoint_json["properties"].is_object(),
        "expected a `properties` object: {endpoint_json}"
    );
    assert!(
        endpoint_json["contract"].is_object(),
        "expected a `contract` object: {endpoint_json}"
    );
    let cells = endpoint_json["cells"]
        .as_array()
        .expect("`cells` must be an array");
    assert!(!cells.is_empty(), "daily_events must have plan cells");

    // Cross-consumer parity check: an independently-assembled call into the
    // same shared builder must produce byte-for-byte identical JSON.
    let independent_json = assemble_diagnostics_independently(&project_dir, "daily_events");

    assert_eq!(
        endpoint_json, independent_json,
        "the HTTP endpoint must ship `build_model_diagnostics`'s output verbatim — \
         no reshaping or dropped fields (thin-consumer boundary)"
    );
}

/// `GET /api/models/<unknown>/diagnostics` must 404, matching the existing
/// `/api/models/:name` convention exactly (`docs/specs/ui_model_diagnostics.md`
/// §Surface "UI REST API": "404 if the model doesn't resolve").
#[tokio::test]
async fn diagnostics_endpoint_404_for_unknown_model() {
    let project_dir = timeseries_project_dir();
    let state = state_from_project(project_dir);
    let app = test_app(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/models/does_not_exist/diagnostics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
