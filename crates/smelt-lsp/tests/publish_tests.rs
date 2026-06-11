//! Tests for the `smelt/publishTests` notification.
//!
//! The LSP must push a `smelt/publishTests` notification after workspace
//! initialization, listing all test models so the VSCode TestController can
//! discover tests without scanning files itself.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tower_lsp::{LspService, Server};

use smelt_lsp::Backend;

// ---------------------------------------------------------------------------
// Protocol helpers (copied from e2e.rs pattern)
// ---------------------------------------------------------------------------

fn encode_message(msg: &Value) -> Vec<u8> {
    let body = serde_json::to_string(msg).unwrap();
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body).into_bytes()
}

async fn read_message(stream: &mut DuplexStream) -> Option<Value> {
    let mut header_buf = Vec::new();
    loop {
        let byte = match stream.read_u8().await {
            Ok(b) => b,
            Err(_) => return None,
        };
        header_buf.push(byte);
        if header_buf.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let header_str = String::from_utf8_lossy(&header_buf);
    let content_length: usize = header_str
        .lines()
        .find_map(|line| {
            let line = line.trim();
            if line.to_lowercase().starts_with("content-length:") {
                line.split(':').nth(1)?.trim().parse().ok()
            } else {
                None
            }
        })
        .expect("Missing Content-Length header");

    let mut body = vec![0u8; content_length];
    stream.read_exact(&mut body).await.ok()?;
    serde_json::from_slice(&body).ok()
}

async fn read_message_timeout(stream: &mut DuplexStream, timeout_ms: u64) -> Option<Value> {
    tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        read_message(stream),
    )
    .await
    .ok()
    .flatten()
}

// ---------------------------------------------------------------------------
// Workspace fixture
// ---------------------------------------------------------------------------

struct TestWorkspace {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::fs::create_dir_all(path.join("models")).unwrap();
        std::fs::create_dir_all(path.join("tests")).unwrap();
        std::fs::write(
            path.join("smelt.yml"),
            "name: test_pub\nversion: 1\npaths:\n  - models\n  - tests\n",
        )
        .unwrap();
        std::fs::write(path.join("models/simple.sql"), "SELECT 1 AS x\n").unwrap();
        Self { _dir: dir, path }
    }

    fn add_test_model(&self, name: &str, target_model: &str) {
        let content = format!(
            "--- name: {name} ---\nmaterialization: test\ntest:\n  model: {target_model}\n  expect:\n    - {{x: 1}}\n---\n"
        );
        std::fs::write(self.path.join("tests").join(format!("{name}.sql")), content).unwrap();
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

// ---------------------------------------------------------------------------
// LSP client helper
// ---------------------------------------------------------------------------

async fn start_lsp_and_init(workspace: &TestWorkspace) -> (DuplexStream, DuplexStream) {
    let (service, socket) = LspService::new(Backend::new);
    let (server_stdin_read, mut client_tx) = tokio::io::duplex(64 * 1024);
    let (mut client_rx_read, server_stdout_write) = tokio::io::duplex(64 * 1024);

    tokio::spawn(async move {
        Server::new(server_stdin_read, server_stdout_write, socket)
            .serve(service)
            .await;
    });

    let workspace_uri = format!("file://{}", workspace.path().display());
    let init_msg = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "processId": null,
            "capabilities": {
                "workspace": { "workspaceFolders": true }
            },
            "workspaceFolders": [{ "uri": workspace_uri, "name": "test" }]
        }
    });
    client_tx
        .write_all(&encode_message(&init_msg))
        .await
        .unwrap();

    // Wait for the initialize response before sending initialized.
    loop {
        let msg = read_message_timeout(&mut client_rx_read, 5000)
            .await
            .expect("timeout waiting for initialize response");
        if msg.get("id").and_then(|v| v.as_i64()) == Some(1) {
            break; // got the response
        }
    }

    // Send initialized notification
    let initialized = json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    });
    client_tx
        .write_all(&encode_message(&initialized))
        .await
        .unwrap();

    (client_rx_read, client_tx)
}

/// Collect all notifications of a given method within timeout_ms.
/// Responds with success to any server-originated requests (e.g.
/// client/registerCapability) so the server doesn't block awaiting them.
async fn collect_notifications(
    rx: &mut DuplexStream,
    tx: &mut tokio::io::DuplexStream,
    method: &str,
    timeout_ms: u64,
) -> Vec<Value> {
    let mut results = Vec::new();
    while let Some(msg) = read_message_timeout(rx, timeout_ms).await {
        // If the server sent a request (has id + method), reply with success.
        if msg.get("id").is_some() && msg.get("method").is_some() {
            let id = msg["id"].clone();
            let response = json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": null
            });
            let _ = tx.write_all(&encode_message(&response)).await;
        }
        if msg.get("method").and_then(|m| m.as_str()) == Some(method) {
            results.push(msg);
        }
    }
    results
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// After initialization of a workspace containing a test model, the LSP must
/// send a `smelt/publishTests` notification.
#[tokio::test]
async fn lsp_publishes_tests_after_init() {
    let ws = TestWorkspace::new();
    ws.add_test_model("test_simple", "simple");

    let (mut rx, mut tx) = start_lsp_and_init(&ws).await;

    let notifications = collect_notifications(&mut rx, &mut tx, "smelt/publishTests", 3000).await;

    assert!(
        !notifications.is_empty(),
        "LSP must send smelt/publishTests notification after init"
    );
}

/// The `smelt/publishTests` notification params must contain a `tests` array.
#[tokio::test]
async fn publish_tests_params_have_tests_array() {
    let ws = TestWorkspace::new();
    ws.add_test_model("test_simple", "simple");

    let (mut rx, mut tx) = start_lsp_and_init(&ws).await;

    let notifications = collect_notifications(&mut rx, &mut tx, "smelt/publishTests", 3000).await;
    let notif = notifications
        .last()
        .expect("expected at least one smelt/publishTests notification");

    let tests = notif["params"]["tests"]
        .as_array()
        .expect("params.tests must be an array");

    assert!(
        !tests.is_empty(),
        "tests array must contain the discovered test model; got params: {}",
        notif["params"]
    );
}

/// Each test entry must have `name`, `uri`, and `line` fields.
#[tokio::test]
async fn publish_tests_entries_have_required_fields() {
    let ws = TestWorkspace::new();
    ws.add_test_model("test_simple", "simple");

    let (mut rx, mut tx) = start_lsp_and_init(&ws).await;

    let notifications = collect_notifications(&mut rx, &mut tx, "smelt/publishTests", 3000).await;
    let notif = notifications
        .last()
        .expect("expected at least one smelt/publishTests notification");

    let tests = notif["params"]["tests"]
        .as_array()
        .expect("params.tests must be an array");
    let entry = &tests[0];

    assert!(
        entry.get("name").is_some(),
        "test entry must have 'name'; got: {entry}"
    );
    assert!(
        entry.get("uri").is_some(),
        "test entry must have 'uri'; got: {entry}"
    );
    assert!(
        entry.get("line").is_some(),
        "test entry must have 'line'; got: {entry}"
    );

    assert_eq!(
        entry["name"].as_str(),
        Some("test_simple"),
        "test name must match model name; got: {entry}"
    );
    assert!(
        entry["uri"]
            .as_str()
            .unwrap_or("")
            .ends_with("test_simple.sql"),
        "test uri must point to the test file; got: {entry}"
    );

    // line must be 0 — the `--- name:` delimiter is the first line of the file,
    // so the gutter icon lands at the declaration, not after the closing `---`.
    assert_eq!(
        entry["line"].as_u64(),
        Some(0),
        "line must be the delimiter line (0); got: {entry}"
    );
}
