use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::http::header;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use rust_embed::Embed;
use tower_http::cors::CorsLayer;

use crate::api;
use crate::types::*;

#[derive(Embed)]
#[folder = "../../ui/dist"]
struct Assets;

pub struct AppState {
    pub project: ProjectResponse,
    pub graph: GraphResponse,
    pub models: HashMap<String, ModelDetailResponse>,
}

pub async fn start_server(
    project: ProjectResponse,
    graph: GraphResponse,
    models: HashMap<String, ModelDetailResponse>,
    port: u16,
    host: &str,
) -> Result<()> {
    let state = Arc::new(AppState {
        project,
        graph,
        models,
    });

    let app = Router::new()
        .route("/api/project", get(api::get_project))
        .route("/api/graph", get(api::get_graph))
        .route("/api/models/{name}", get(api::get_model))
        .fallback(static_handler)
        // Permissive CORS for dev mode (Vite dev server proxies to this port)
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("{}:{}", host, port);
    println!("Starting UI server at http://{}:{}", host, port);
    println!("Serving a snapshot of your project — restart to pick up changes");

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind to {}", addr))?;

    axum::serve(listener, app)
        .await
        .with_context(|| "Server error")?;

    Ok(())
}

async fn static_handler(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Try the exact path first
    if let Some(file) = Assets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        return ([(header::CONTENT_TYPE, mime.as_ref())], file.data).into_response();
    }

    // SPA fallback: serve index.html for non-file paths
    if let Some(file) = Assets::get("index.html") {
        let html = String::from_utf8_lossy(&file.data).into_owned();
        return Html(html).into_response();
    }

    // No embedded assets - show helpful message
    Html(
        r#"<!DOCTYPE html>
<html><body>
<h1>smelt UI</h1>
<p>No frontend assets found. Build the UI first:</p>
<pre>cd ui && npm install && npm run build</pre>
<p>Or use the dev server: <code>cd ui && npm run dev</code></p>
<h2>API Endpoints</h2>
<ul>
<li><a href="/api/project">/api/project</a></li>
<li><a href="/api/graph">/api/graph</a></li>
</ul>
</body></html>"#,
    )
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn test_state() -> Arc<AppState> {
        let mut models = HashMap::new();
        models.insert(
            "test_model".to_string(),
            ModelDetailResponse {
                name: "test_model".to_string(),
                path: "models/test_model.sql".to_string(),
                sql: "SELECT 1".to_string(),
                materialization: Some("view".to_string()),
                tags: vec!["test".to_string()],
                owner: None,
                description: Some("A test model".to_string()),
                refs: vec![],
                columns: vec![],
            },
        );

        Arc::new(AppState {
            project: ProjectResponse {
                name: "test".to_string(),
                version: 1,
                model_count: 1,
                source_count: 0,
            },
            graph: GraphResponse {
                nodes: vec![GraphNode {
                    id: "test_model".to_string(),
                    label: "test_model".to_string(),
                    materialization: Some("view".to_string()),
                    tags: vec!["test".to_string()],
                    description: Some("A test model".to_string()),
                    has_errors: false,
                    node_type: NodeType::Model,
                }],
                edges: vec![],
                sources: vec![],
            },
            models,
        })
    }

    fn test_app() -> Router {
        let state = test_state();
        Router::new()
            .route("/api/project", get(api::get_project))
            .route("/api/graph", get(api::get_graph))
            .route("/api/models/{name}", get(api::get_model))
            .with_state(state)
    }

    #[tokio::test]
    async fn test_get_project() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/project")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let project: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(project["name"], "test");
        assert_eq!(project["model_count"], 1);
    }

    #[tokio::test]
    async fn test_get_graph() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/graph")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let graph: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(graph["nodes"].as_array().unwrap().len(), 1);
        assert_eq!(graph["nodes"][0]["id"], "test_model");
    }

    #[tokio::test]
    async fn test_get_model_found() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/models/test_model")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let model: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(model["name"], "test_model");
        assert_eq!(model["sql"], "SELECT 1");
    }

    #[tokio::test]
    async fn test_get_model_not_found() {
        let app = test_app();
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/models/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
