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
) -> Result<()> {
    let state = Arc::new(AppState {
        project,
        graph,
        models,
    });

    let app = Router::new()
        .route("/api/project", get(api::get_project))
        .route("/api/graph", get(api::get_graph))
        .route("/api/models/:name", get(api::get_model))
        .fallback(static_handler)
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    println!("Starting UI server at http://localhost:{}", port);

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
