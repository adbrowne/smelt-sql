use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::WebSocketUpgrade;
use axum::http::header;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use rust_embed::Embed;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;

use smelt_core::config::Config;
use smelt_core::graph::DependencyGraph;
use smelt_core::SourcesConfig;

use crate::api;
use crate::run_manager::RunManager;
use crate::types::RunProgressEvent;

#[derive(Embed)]
#[folder = "../../ui/dist"]
struct Assets;

/// Events pushed to WebSocket clients.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "type")]
pub enum ChangeEvent {
    /// Model files changed on disk — clients should refetch.
    #[serde(rename = "models_updated")]
    ModelsUpdated,
}

pub struct AppState {
    pub db: Arc<tokio::sync::Mutex<smelt_db::Database>>,
    pub config: Arc<Config>,
    pub sources: Arc<Option<SourcesConfig>>,
    pub graph: Arc<tokio::sync::Mutex<DependencyGraph>>,
    pub project_dir: PathBuf,
    pub change_tx: broadcast::Sender<ChangeEvent>,
    pub run_manager: Arc<RunManager>,
    pub run_event_tx: broadcast::Sender<RunProgressEvent>,
}

pub async fn start_server(
    db: smelt_db::Database,
    config: Config,
    sources: Option<SourcesConfig>,
    graph: DependencyGraph,
    project_dir: PathBuf,
    port: u16,
    host: &str,
) -> Result<()> {
    let (change_tx, _) = broadcast::channel::<ChangeEvent>(64);
    let (run_event_tx, _) = broadcast::channel::<RunProgressEvent>(256);

    let run_manager = Arc::new(RunManager::new(run_event_tx.clone(), project_dir.clone()));

    let state = Arc::new(AppState {
        db: Arc::new(tokio::sync::Mutex::new(db)),
        config: Arc::new(config.clone()),
        sources: Arc::new(sources),
        graph: Arc::new(tokio::sync::Mutex::new(graph)),
        project_dir: project_dir.clone(),
        change_tx: change_tx.clone(),
        run_manager,
        run_event_tx: run_event_tx.clone(),
    });

    // Start file watcher
    let watcher_state = state.clone();
    let model_paths: Vec<PathBuf> = config
        .model_paths
        .iter()
        .map(|p| project_dir.join(p))
        .collect();
    crate::watcher::start_watcher(watcher_state, model_paths, project_dir.clone())?;

    let app = Router::new()
        .route("/api/project", get(api::get_project))
        .route("/api/graph", get(api::get_graph))
        .route("/api/models/{name}", get(api::get_model))
        .route("/api/resolve", post(api::post_resolve))
        .route("/api/run/plan", post(api::post_run_plan))
        .route("/api/run/execute", post(api::post_run_execute))
        .route("/api/run/cancel", post(api::post_run_cancel))
        .route("/api/run/status", get(api::get_run_status))
        .route("/api/runs", get(api::get_runs))
        .route("/api/runs/{id}", get(api::get_run))
        .route("/ws", get(ws_handler))
        .fallback(static_handler)
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("{}:{}", host, port);
    println!("Starting UI server at http://{}:{}", host, port);
    println!("Live mode — file changes will auto-update the UI");

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .with_context(|| format!("Failed to bind to {}", addr))?;

    axum::serve(listener, app)
        .await
        .with_context(|| "Server error")?;

    Ok(())
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Response {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: Arc<AppState>) {
    let mut change_rx = state.change_tx.subscribe();
    let mut run_rx = state.run_event_tx.subscribe();

    loop {
        tokio::select! {
            result = change_rx.recv() => {
                match result {
                    Ok(event) => {
                        let json = serde_json::to_string(&event).unwrap_or_default();
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("WebSocket client lagged by {} change events", n);
                        let event = ChangeEvent::ModelsUpdated;
                        let json = serde_json::to_string(&event).unwrap_or_default();
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            result = run_rx.recv() => {
                match result {
                    Ok(event) => {
                        let json = serde_json::to_string(&event).unwrap_or_default();
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("WebSocket client lagged by {} run events", n);
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn static_handler(uri: axum::http::Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    if let Some(file) = Assets::get(path) {
        let mime = mime_guess::from_path(path).first_or_octet_stream();
        return ([(header::CONTENT_TYPE, mime.as_ref())], file.data).into_response();
    }

    // SPA fallback: serve index.html for non-file paths
    if let Some(file) = Assets::get("index.html") {
        let html = String::from_utf8_lossy(&file.data).into_owned();
        return Html(html).into_response();
    }

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
    use smelt_core::config::Materialization;
    use smelt_core::discovery::ModelFile;
    use smelt_db::Inputs;
    use std::collections::HashMap;
    use tower::ServiceExt;

    fn test_state() -> Arc<AppState> {
        let models = vec![ModelFile {
            name: "test_model".to_string(),
            path: "models/test_model.sql".into(),
            content: "SELECT 1".to_string(),
            refs: vec![],
            parse_errors: vec![],
            metadata: None,
            model_id: smelt_core::ModelId::from_path("models/test_model.sql".into()),
        }];

        let graph = DependencyGraph::build(models.clone(), None).unwrap();

        let mut db = smelt_db::Database::default();
        let project_root = PathBuf::from("/test");
        db.set_project_sources_yaml(project_root.clone(), Arc::new(String::new()));
        db.set_all_project_roots(Arc::new(vec![project_root.clone()]));
        let mut file_paths = Vec::new();
        for model in &models {
            db.set_file_text(model.path.clone(), Arc::new(model.content.clone()));
            db.set_file_project_root(model.path.clone(), project_root.clone());
            file_paths.push(model.path.clone());
        }
        db.set_all_files(Arc::new(file_paths));

        let (change_tx, _) = broadcast::channel(16);
        let (run_event_tx, _) = broadcast::channel(16);

        let mut targets = HashMap::new();
        targets.insert(
            "dev".to_string(),
            smelt_core::config::Target {
                target_type: "duckdb".to_string(),
                database: Some("test.duckdb".to_string()),
                schema: "main".to_string(),
                connect_url: None,
                catalog: None,
            },
        );

        let project_root_clone = project_root.clone();
        Arc::new(AppState {
            db: Arc::new(tokio::sync::Mutex::new(db)),
            config: Arc::new(Config {
                name: "test".to_string(),
                version: 1,
                model_paths: vec!["models".to_string()],
                seed_paths: vec!["seeds".to_string()],
                targets,
                default_materialization: Materialization::View,
                models: HashMap::new(),
                python: None,
            }),
            sources: Arc::new(None),
            graph: Arc::new(tokio::sync::Mutex::new(graph)),
            project_dir: project_root,
            change_tx,
            run_manager: Arc::new(RunManager::new(run_event_tx.clone(), project_root_clone)),
            run_event_tx,
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
