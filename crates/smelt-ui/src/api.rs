use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;

use crate::server::AppState;
use crate::types::*;

pub async fn get_project(State(state): State<Arc<AppState>>) -> Json<ProjectResponse> {
    Json(state.project.clone())
}

pub async fn get_graph(State(state): State<Arc<AppState>>) -> Json<GraphResponse> {
    Json(state.graph.clone())
}

pub async fn get_model(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> Result<Json<ModelDetailResponse>, StatusCode> {
    state
        .models
        .get(&name)
        .map(|m| Json(m.clone()))
        .ok_or(StatusCode::NOT_FOUND)
}
