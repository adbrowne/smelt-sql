//! Stub-coordinator tests for `TrinoClient`. No test here talks to a live
//! Trino server (`SMELT_TRINO_URL` is never read) — the stub is an `axum`
//! router bound to an ephemeral port, standing in for `/v1/statement` +
//! `nextUri` paging.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use arrow::array::Int64Array;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use smelt_backend::BackendError;
use smelt_backend_trino::{TrinoClient, TrinoClientConfig};

#[derive(Clone)]
struct StubState {
    pages: Arc<Mutex<VecDeque<Value>>>,
    captured_headers: Arc<Mutex<Vec<HeaderMap>>>,
}

async fn page_handler(State(state): State<StubState>, headers: HeaderMap) -> Response {
    state.captured_headers.lock().unwrap().push(headers);
    let mut pages = state.pages.lock().unwrap();
    match pages.pop_front() {
        Some(page) => Json(page).into_response(),
        None => (StatusCode::INTERNAL_SERVER_ERROR, "stub: no more pages").into_response(),
    }
}

async fn spawn_stub() -> (String, StubState) {
    let state = StubState {
        pages: Arc::new(Mutex::new(VecDeque::new())),
        captured_headers: Arc::new(Mutex::new(Vec::new())),
    };
    let app = Router::new()
        .route("/v1/statement", post(page_handler))
        .route("/v1/statement/next", get(page_handler))
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("stub server");
    });
    (format!("http://{addr}"), state)
}

async fn spawn_5xx_stub() -> String {
    async fn always_503() -> Response {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            [("content-type", "text/html")],
            "<html><body>service unavailable</body></html>",
        )
            .into_response()
    }
    let app = Router::new().route("/v1/statement", post(always_503));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("stub server");
    });
    format!("http://{addr}")
}

fn config(base_url: &str) -> TrinoClientConfig {
    TrinoClientConfig {
        base_url: base_url.to_string(),
        user: "smelt".to_string(),
        catalog: "iceberg".to_string(),
        schema: "default".to_string(),
        password: None,
    }
}

#[tokio::test]
async fn follows_next_uri_to_the_end() {
    let (base_url, state) = spawn_stub().await;
    let next = format!("{base_url}/v1/statement/next");
    state.pages.lock().unwrap().extend([
        json!({
            "id": "q1", "nextUri": next, "stats": {"state": "RUNNING"},
            "columns": [{"name": "id", "type": "bigint"}],
            "data": [[1]],
        }),
        json!({
            "id": "q1", "nextUri": next, "stats": {"state": "RUNNING"},
            "columns": [{"name": "id", "type": "bigint"}],
            "data": [[2]],
        }),
        json!({
            "id": "q1", "stats": {"state": "FINISHED"},
            "columns": [{"name": "id", "type": "bigint"}],
            "data": [[3]],
        }),
    ]);

    let client = TrinoClient::new(config(&base_url));
    let batches = client.execute("select id from t").await.unwrap();

    let values: Vec<i64> = batches
        .iter()
        .flat_map(|b| {
            b.column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .iter()
                .map(|v| v.unwrap())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(values, vec![1, 2, 3]);
}

#[tokio::test]
async fn queued_pages_with_no_columns_are_not_an_empty_result() {
    let (base_url, state) = spawn_stub().await;
    let next = format!("{base_url}/v1/statement/next");
    state.pages.lock().unwrap().extend([
        json!({
            "id": "q1", "nextUri": next, "stats": {"state": "QUEUED"},
        }),
        json!({
            "id": "q1", "stats": {"state": "FINISHED"},
            "columns": [{"name": "id", "type": "bigint"}],
            "data": [[42]],
        }),
    ]);

    let client = TrinoClient::new(config(&base_url));
    let batches = client.execute("select id from t").await.unwrap();

    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 1);
    let ids = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .unwrap();
    assert_eq!(ids.value(0), 42);
}

#[tokio::test]
async fn an_error_page_is_never_a_silent_empty_result() {
    let (base_url, state) = spawn_stub().await;
    state.pages.lock().unwrap().extend([json!({
        "id": "q1", "stats": {"state": "FAILED"},
        "columns": [{"name": "id", "type": "bigint"}],
        "data": [],
        "error": {
            "message": "table foo.bar not found",
            "errorCode": 1,
            "errorName": "TABLE_NOT_FOUND",
            "errorType": "USER_ERROR",
        },
    })]);

    let client = TrinoClient::new(config(&base_url));
    let err = client.execute("select 1 from foo.bar").await.unwrap_err();
    assert!(matches!(err, BackendError::NotFound { .. }));
}

#[tokio::test]
async fn sends_the_trino_session_headers() {
    let (base_url, state) = spawn_stub().await;
    state.pages.lock().unwrap().push_back(json!({
        "id": "q1", "stats": {"state": "FINISHED"},
        "columns": [{"name": "id", "type": "bigint"}],
        "data": [[1]],
    }));

    let mut cfg = config(&base_url);
    cfg.password = Some("hunter2".to_string());
    let client = TrinoClient::new(cfg);
    client.execute("select 1").await.unwrap();

    let captured = state.captured_headers.lock().unwrap();
    let headers = captured.first().expect("one request captured");
    assert_eq!(headers.get("x-trino-user").unwrap(), "smelt");
    assert_eq!(headers.get("x-trino-catalog").unwrap(), "iceberg");
    assert_eq!(headers.get("x-trino-schema").unwrap(), "default");
    let auth = headers
        .get("authorization")
        .expect("authorization header present")
        .to_str()
        .unwrap();
    assert!(auth.starts_with("Basic "));
}

#[tokio::test]
async fn a_refused_connection_is_connection_failed() {
    // Bind then drop a listener to get a port nothing is listening on.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    drop(listener);

    let client = TrinoClient::new(config(&format!("http://{addr}")));
    let err = client.execute("select 1").await.unwrap_err();
    assert!(matches!(err, BackendError::ConnectionFailed { .. }));
    assert!(!err.to_string().contains("hunter2"));
}

#[tokio::test]
async fn an_http_5xx_is_not_parsed_as_a_result_page() {
    let base_url = spawn_5xx_stub().await;
    let client = TrinoClient::new(config(&base_url));
    let err = client.execute("select 1").await.unwrap_err();
    let message = err.to_string();
    assert!(message.contains("503"));
}
