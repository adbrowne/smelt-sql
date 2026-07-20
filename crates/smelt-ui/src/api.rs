use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

use smelt_state::file_store::FileStore;

use crate::build;
use crate::server::AppState;
use crate::types::*;

/// Query params accepted by the run-history endpoints. `target` defaults to
/// `"dev"` — the same default `RunExecuteRequest::target` and the CLI's
/// `--target` use — since `.smelt/` state is partitioned per target
/// (`docs/specs/run_state.md` §"`.smelt/` directory layout") and history
/// queries need to name which target's state they're reading.
#[derive(Deserialize)]
pub struct TargetQuery {
    #[serde(default = "default_target_query")]
    target: String,
}

fn default_target_query() -> String {
    "dev".to_string()
}

/// Validate that `target` is a configured target (fail-loud rather than
/// silently reading — and reporting empty — state for a target that
/// doesn't exist in `smelt.yml`).
fn require_known_target(state: &AppState, target: &str) -> Result<(), (StatusCode, String)> {
    if state.config.targets.contains_key(target) {
        Ok(())
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Target '{}' not found in smelt.yml. Available targets: {}",
                target,
                state
                    .config
                    .targets
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ))
    }
}

pub async fn get_project(State(state): State<Arc<AppState>>) -> Json<ProjectResponse> {
    let graph = state.graph.lock().await;
    let response =
        build::build_project_response(&state.config, &graph, state.sources.as_ref().as_ref());
    Json(response)
}

pub async fn get_graph(State(state): State<Arc<AppState>>) -> Json<GraphResponse> {
    let graph = state.graph.lock().await;
    let response = build::build_graph_response(&graph, &state.config);
    Json(response)
}

pub async fn get_model(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<ModelDetailResponse>, StatusCode> {
    let graph = state.graph.lock().await;
    let db = state.db.lock().await;
    let details = build::build_model_details(&graph, &state.config, &db);
    details
        .get(&name)
        .cloned()
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

pub async fn post_resolve(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ResolveRequest>,
) -> Result<Json<ResolveResponse>, impl IntoResponse> {
    let graph = state.graph.lock().await;
    match build::resolve_selectors(&graph, &state.config, &request) {
        Ok(response) => Ok(Json(response)),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

pub async fn post_run_plan(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RunPlanRequest>,
) -> Result<Json<RunPlanResponse>, impl IntoResponse> {
    let graph = state.graph.lock().await;
    match build::build_run_plan(&graph, &state.config, &request) {
        Ok(plan) => Ok(Json(plan)),
        Err(e) => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

// --- Run Execution Endpoints ---

pub async fn post_run_execute(
    State(state): State<Arc<AppState>>,
    Json(request): Json<RunExecuteRequest>,
) -> Result<Json<serde_json::Value>, impl IntoResponse> {
    match state
        .run_manager
        .execute(
            request,
            state.config.clone(),
            state.sources.clone(),
            state.graph.clone(),
            state.db.clone(),
        )
        .await
    {
        Ok(run_id) => Ok(Json(serde_json::json!({ "run_id": run_id }))),
        Err(msg) => Err((StatusCode::CONFLICT, msg.to_string())),
    }
}

pub async fn post_run_cancel(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, impl IntoResponse> {
    if state.run_manager.cancel().await {
        Ok(Json(serde_json::json!({ "cancelled": true })))
    } else {
        Err((StatusCode::BAD_REQUEST, "No run in progress".to_string()))
    }
}

pub async fn get_run_status(State(state): State<Arc<AppState>>) -> Json<RunStatusResponse> {
    Json(state.run_manager.status().await)
}

pub async fn get_runs(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TargetQuery>,
) -> Result<Json<Vec<RunHistoryEntry>>, impl IntoResponse> {
    require_known_target(&state, &query.target)?;
    let file_store = FileStore::new(&state.project_dir, &query.target);
    match file_store.load_runs(Some(50)) {
        Ok(manifests) => {
            let entries: Vec<RunHistoryEntry> = manifests
                .into_iter()
                .map(|m| {
                    let models = m
                        .models
                        .into_iter()
                        .map(|(name, record)| {
                            (
                                name,
                                RunModelSummary {
                                    strategy: record.strategy,
                                    row_count: record.row_count,
                                    duration_ms: record.duration_ms,
                                    time_range: record.time_range.map(|tr| PlanTimeRange {
                                        start: tr.start,
                                        end: tr.end,
                                    }),
                                },
                            )
                        })
                        .collect::<std::collections::HashMap<_, _>>();
                    RunHistoryEntry {
                        run_id: m.run_id,
                        started_at: m.started_at,
                        completed_at: m.completed_at,
                        model_count: models.len(),
                        models,
                    }
                })
                .collect();
            Ok(Json(entries))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load run history: {}", e),
        )),
    }
}

pub async fn get_run(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<TargetQuery>,
) -> Result<Json<RunHistoryEntry>, impl IntoResponse> {
    require_known_target(&state, &query.target)?;
    let file_store = FileStore::new(&state.project_dir, &query.target);
    match file_store.load_run(&id) {
        Ok(Some(m)) => {
            let models = m
                .models
                .into_iter()
                .map(|(name, record)| {
                    (
                        name,
                        RunModelSummary {
                            strategy: record.strategy,
                            row_count: record.row_count,
                            duration_ms: record.duration_ms,
                            time_range: record.time_range.map(|tr| PlanTimeRange {
                                start: tr.start,
                                end: tr.end,
                            }),
                        },
                    )
                })
                .collect::<std::collections::HashMap<_, _>>();
            Ok(Json(RunHistoryEntry {
                run_id: m.run_id,
                started_at: m.started_at,
                completed_at: m.completed_at,
                model_count: models.len(),
                models,
            }))
        }
        Ok(None) => Err((StatusCode::NOT_FOUND, "Run not found".to_string())),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to load run: {}", e),
        )),
    }
}
