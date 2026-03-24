use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use crate::build;
use crate::server::AppState;
use crate::types::*;

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
