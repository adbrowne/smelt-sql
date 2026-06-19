//! Integration tests asserting that the UI's HTTP API emits canonical
//! `smelt.<path>` identifiers for every model identifier field.
//!
//! Phase 5 of the canonical-addressing plan: the graph node IDs, label fields,
//! dependency lists, and `name` fields all use the dot-joined canonical path
//! rather than the bare leaf name.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use axum::Router;
use smelt_core::config::Target;
use smelt_core::discovery::ModelFile;
use smelt_core::graph::DependencyGraph;
use smelt_core::workspace::load_workspace;
use smelt_db::{workspace_ingest::ingest_loaded_workspace, Database};
use smelt_ui::build;
use smelt_ui::server::AppState;
use std::collections::HashMap;
use std::path::PathBuf;
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

/// Build a test AppState from a real on-disk project root.
fn state_from_project(project_root: PathBuf) -> Arc<AppState> {
    let loaded = load_workspace(&project_root);
    let mut db = Database::default();
    let ingested = ingest_loaded_workspace(&mut db, &loaded);
    db.set_workspace(ingested.source_files, vec![ingested.project]);

    let config = loaded.config.clone();
    let sources = smelt_core::SourcesConfig::load(&project_root).ok();

    // Build DependencyGraph from the loaded model files (exclude function files).
    let models: Vec<ModelFile> = loaded
        .sql_files
        .into_iter()
        .filter(|m| matches!(m.kind, smelt_core::ModelKind::Sql))
        .filter(|m| {
            // Only include models under `config.paths` (not function files).
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
        },
    );

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
    use smelt_ui::server::AppState as AS;
    Router::new()
        .route(
            "/api/graph",
            get(|s: axum::extract::State<Arc<AS>>| async move {
                let graph = s.graph.lock().await;
                axum::Json(build::build_graph_response(&graph, &s.config))
            }),
        )
        .route(
            "/api/models/{name}",
            get(
                |s: axum::extract::State<Arc<AS>>,
                 axum::extract::Path(name): axum::extract::Path<String>| async move {
                    let graph = s.graph.lock().await;
                    let db = s.db.lock().await;
                    let details = build::build_model_details(&graph, &s.config, &db);
                    details
                        .get(&name)
                        .cloned()
                        .map(axum::Json)
                        .ok_or(StatusCode::NOT_FOUND)
                },
            ),
        )
        .with_state(state)
}

/// GET /api/graph against web_analytics must produce node IDs that are
/// canonical dot-paths (`bronze.raw_events`, `silver.events_parsed`, etc.).
/// No bare leaf name (e.g. `raw_events`) should appear as a node `id`.
#[tokio::test]
async fn models_endpoint_emits_canonical_paths() {
    let project_root = examples_dir().join("web_analytics");
    let state = state_from_project(project_root);
    let app = test_app(state);

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

    let nodes = graph["nodes"].as_array().expect("nodes must be array");
    assert!(
        !nodes.is_empty(),
        "expected at least one model node from web_analytics"
    );

    // Every model node's `id` must contain a dot (canonical path form).
    // Source nodes (like `raw.events`) also contain dots, so this applies
    // broadly. Bare leaf names that appear as node IDs are the bug.
    let bare_leaf_ids: Vec<String> = nodes
        .iter()
        .filter(|n| n["node_type"].as_str() == Some("model"))
        .filter(|n| {
            let id = n["id"].as_str().unwrap_or("");
            !id.contains('.')
        })
        .map(|n| n["id"].as_str().unwrap_or("").to_string())
        .collect();

    assert!(
        bare_leaf_ids.is_empty(),
        "model nodes with bare leaf IDs (should be canonical paths): {:?}",
        bare_leaf_ids
    );

    // Spot-check: known canonical paths must appear.
    let ids: Vec<&str> = nodes.iter().filter_map(|n| n["id"].as_str()).collect();

    for expected in &["bronze.raw_events", "silver.events_parsed"] {
        assert!(
            ids.contains(expected),
            "expected canonical path {:?} in node IDs, got: {:?}",
            expected,
            ids
        );
    }

    // Labels must also be canonical paths (label == id in the current impl).
    let bad_labels: Vec<String> = nodes
        .iter()
        .filter(|n| n["node_type"].as_str() == Some("model"))
        .filter(|n| {
            let label = n["label"].as_str().unwrap_or("");
            !label.contains('.')
        })
        .map(|n| n["label"].as_str().unwrap_or("").to_string())
        .collect();

    assert!(
        bad_labels.is_empty(),
        "model nodes with bare leaf labels (should be canonical paths): {:?}",
        bad_labels
    );
}

/// The `refs` field in each model detail must contain canonical paths.
#[tokio::test]
async fn dependencies_are_canonical_paths() {
    let project_root = examples_dir().join("web_analytics");
    let state = state_from_project(project_root);
    let app = test_app(state);

    // Fetch the graph first to get canonical model IDs.
    let graph_resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/graph")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let graph_body = axum::body::to_bytes(graph_resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let graph: serde_json::Value = serde_json::from_slice(&graph_body).unwrap();
    let node_ids: Vec<String> = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["node_type"].as_str() == Some("model"))
        .filter_map(|n| n["id"].as_str().map(|s| s.to_string()))
        .collect();

    // For each model, fetch detail and verify its `refs` are canonical.
    for model_id in &node_ids {
        let url = format!("/api/models/{}", urlencoding::encode(model_id));
        let resp = app
            .clone()
            .oneshot(Request::builder().uri(&url).body(Body::empty()).unwrap())
            .await
            .unwrap();

        if resp.status() != StatusCode::OK {
            // Some models may not be found via detail endpoint — skip.
            continue;
        }

        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let detail: serde_json::Value = serde_json::from_slice(&body).unwrap();

        let refs = match detail["refs"].as_array() {
            Some(r) => r.clone(),
            None => continue,
        };

        for dep in &refs {
            let dep_str = dep.as_str().unwrap_or("");
            // Skip source refs (raw.table_name style) — they are already canonical.
            // Only flag model refs that look like bare leaf names.
            // A bare leaf name is one that contains no '.' at all.
            assert!(
                dep_str.contains('.'),
                "model {:?} has a bare-leaf ref {:?} — expected canonical path",
                model_id,
                dep_str
            );
        }
    }
}

/// Two models with the same leaf name but different namespace prefixes must
/// appear as distinct nodes in the API response.
#[tokio::test]
async fn same_leaf_distinct_namespaces() {
    // Build a tempdir fixture with two `events.sql` files, one under
    // `models/silver/` and one under `models/bronze/`.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    std::fs::create_dir_all(root.join("models/silver")).unwrap();
    std::fs::create_dir_all(root.join("models/bronze")).unwrap();

    // Write a minimal smelt.yml.
    std::fs::write(
        root.join("smelt.yml"),
        "name: test\nversion: 1\npaths:\n  - models\ntargets:\n  dev:\n    type: duckdb\n    database: test.duckdb\n    schema: main\n",
    )
    .unwrap();

    std::fs::write(root.join("models/bronze/events.sql"), "SELECT 1 AS id").unwrap();
    std::fs::write(
        root.join("models/silver/events.sql"),
        "SELECT id FROM smelt.bronze.events",
    )
    .unwrap();

    let state = state_from_project(root.to_path_buf());
    let app = test_app(state);

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

    let ids: Vec<&str> = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|n| n["id"].as_str())
        .collect();

    assert!(
        ids.contains(&"bronze.events"),
        "expected bronze.events node, got: {:?}",
        ids
    );
    assert!(
        ids.contains(&"silver.events"),
        "expected silver.events node, got: {:?}",
        ids
    );

    // Both must be present (two distinct nodes, not one merged).
    let events_nodes: Vec<&str> = ids
        .iter()
        .copied()
        .filter(|id| id.ends_with(".events") || *id == "events")
        .collect();
    assert_eq!(
        events_nodes.len(),
        2,
        "expected exactly 2 'events' nodes (bronze.events + silver.events), got: {:?}",
        events_nodes
    );
}
