//! LSP-level surfacing of the pre-run definition-change diagnostic
//! (`docs/specs/definition_deltas.md` §Detection): `MaintenanceSkeletonChanged`
//! now derives from a real `ProjectInput::deployed_columns` Salsa input,
//! populated at workspace-loading time and kept fresh by a
//! `.smelt/targets/*/schemas/*.json` watch glob. Drives the REAL `Backend`
//! (via tower-lsp + DuplexStream, same harness as `example_workspaces.rs` —
//! duplicated here rather than shared, per that file's own precedent).

use std::path::Path;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tower_lsp::{LspService, Server};

use smelt_lsp::Backend;

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

struct TestClient {
    client_rx: DuplexStream,
    client_tx: DuplexStream,
    next_id: i64,
    notifications: Vec<Value>,
}

impl TestClient {
    async fn open_workspace(workspace_dir: &Path) -> Self {
        let (service, socket) = LspService::new(Backend::new);
        let (server_stdin_read, client_tx_write) = tokio::io::duplex(64 * 1024);
        let (client_rx_read, server_stdout_write) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            Server::new(server_stdin_read, server_stdout_write, socket)
                .serve(service)
                .await;
        });

        let mut client = Self {
            client_rx: client_rx_read,
            client_tx: client_tx_write,
            next_id: 1,
            notifications: Vec::new(),
        };

        let workspace_uri = format!("file://{}", workspace_dir.display());
        let init_result = client
            .send_request(
                "initialize",
                json!({
                    "processId": null,
                    "capabilities": { "workspace": { "workspaceFolders": true } },
                    "workspaceFolders": [{ "uri": workspace_uri, "name": "test" }]
                }),
            )
            .await;
        assert!(
            init_result.get("capabilities").is_some(),
            "initialize should return capabilities"
        );
        client.send_notification("initialized", json!({})).await;
        client
    }

    async fn send_request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        self.client_tx
            .write_all(&encode_message(&msg))
            .await
            .unwrap();
        loop {
            let response = read_message_timeout(&mut self.client_rx, 60000)
                .await
                .unwrap_or_else(|| {
                    panic!("Timeout waiting for response to {} (id={})", method, id)
                });
            if response.get("id").and_then(|v| v.as_i64()) == Some(id) {
                if let Some(error) = response.get("error") {
                    panic!("LSP error for {}: {}", method, error);
                }
                return response.get("result").cloned().unwrap_or(Value::Null);
            } else {
                self.notifications.push(response);
            }
        }
    }

    async fn send_notification(&mut self, method: &str, params: Value) {
        let msg = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        self.client_tx
            .write_all(&encode_message(&msg))
            .await
            .unwrap();
    }

    async fn open_file(&mut self, path: &Path) -> Result<(), std::io::Error> {
        let text = std::fs::read_to_string(path)?;
        let uri = format!("file://{}", path.display());
        self.send_notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": uri,
                    "languageId": "sql",
                    "version": 1,
                    "text": text
                }
            }),
        )
        .await;
        Ok(())
    }

    async fn did_change_watched_file(&mut self, path: &Path, change_type: i64) {
        let uri = format!("file://{}", path.display());
        self.send_notification(
            "workspace/didChangeWatchedFiles",
            json!({
                "changes": [{ "uri": uri, "type": change_type }]
            }),
        )
        .await;
    }

    /// Drain `publishDiagnostics` notifications and return `(uri, diagnostics)`
    /// pairs. Waits up to `timeout_ms` for stragglers.
    async fn collect_diagnostics(
        &mut self,
        timeout_ms: u64,
    ) -> Vec<(String, Vec<lsp_types::Diagnostic>)> {
        let mut results = Vec::new();
        let buffered = std::mem::take(&mut self.notifications);
        for notif in buffered {
            if notif.get("method").and_then(|m| m.as_str())
                == Some("textDocument/publishDiagnostics")
            {
                if let Some(params) = notif.get("params") {
                    let uri = params["uri"].as_str().unwrap_or("").to_string();
                    let diags: Vec<lsp_types::Diagnostic> =
                        serde_json::from_value(params["diagnostics"].clone()).unwrap_or_default();
                    results.push((uri, diags));
                }
            }
        }
        while let Some(msg) = read_message_timeout(&mut self.client_rx, timeout_ms).await {
            if msg.get("method").and_then(|m| m.as_str()) == Some("textDocument/publishDiagnostics")
            {
                if let Some(params) = msg.get("params") {
                    let uri = params["uri"].as_str().unwrap_or("").to_string();
                    let diags: Vec<lsp_types::Diagnostic> =
                        serde_json::from_value(params["diagnostics"].clone()).unwrap_or_default();
                    results.push((uri, diags));
                }
            }
        }
        results
    }

    async fn shutdown(&mut self) {
        let msg = json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": "shutdown",
            "params": null
        });
        self.next_id += 1;
        self.client_tx
            .write_all(&encode_message(&msg))
            .await
            .unwrap();
        let _ = read_message_timeout(&mut self.client_rx, 1000).await;
        self.send_notification("exit", json!(null)).await;
    }
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

const SMELT_YML: &str = "\
name: skeleton_fixture\n\
version: 1\n\
paths:\n  - models\n\
targets:\n  dev:\n    type: duckdb\n    schema: main\n\
default_materialization: view\n\
state:\n  mode: intervals\n";

const EVENTS_SOURCE: &str = "\
description: events\n\
columns:\n\
- name: device_id\n  type: VARCHAR\n\
- name: user_id\n  type: VARCHAR\n\
mutation_profile:\n  kind: append_only\n";

const MODEL_SQL: &str = "---\n\
materialization: table\n\
refresh: incremental\n\
grain: key\n\
---\n\
SELECT device_id, user_id, COUNT(*) AS n \
FROM smelt.sources.events GROUP BY device_id, user_id\n";

fn write_fixture(root: &Path) {
    std::fs::write(root.join("smelt.yml"), SMELT_YML).unwrap();
    std::fs::create_dir_all(root.join("models/sources")).unwrap();
    std::fs::write(root.join("models/sources/events.yml"), EVENTS_SOURCE).unwrap();
    std::fs::write(root.join("models/device_user_counts.sql"), MODEL_SQL).unwrap();
}

fn write_deployed_schema(root: &Path, columns: &[&str]) {
    let store = smelt_state::file_store::FileStore::new(
        root,
        "dev",
        smelt_core::config::StateMode::Intervals,
    );
    store
        .save_schema(&smelt_state::schema_tracking::DeployedSchema {
            model: "device_user_counts".to_string(),
            version: 1,
            deployed_at: chrono::Utc::now(),
            model_hash: "fixture-hash".to_string(),
            columns: columns
                .iter()
                .map(|name| smelt_state::schema_tracking::DeployedColumn {
                    name: name.to_string(),
                    data_type: "VARCHAR".to_string(),
                    nullable: false,
                })
                .collect(),
            definition_sql: String::new(),
        })
        .expect("save deployed schema");
}

fn has_skeleton_changed(diags: &[lsp_types::Diagnostic]) -> bool {
    diags.iter().any(|d| {
        matches!(
            &d.code,
            Some(lsp_types::NumberOrString::String(s)) if s == "maintenance-skeleton-changed"
        )
    })
}

/// A deployed schema snapshot recorded BEFORE the workspace is opened —
/// missing the `user_id` GROUP BY key column the model now adds — makes the
/// real LSP backend publish `maintenance-skeleton-changed` for the model
/// file, with no run and no manual `smelt explain` invocation.
#[tokio::test]
async fn skeleton_change_surfaces_through_the_lsp() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    write_fixture(&root);
    write_deployed_schema(&root, &["device_id", "n"]);

    let model_path = root.join("models/device_user_counts.sql");
    let mut client = TestClient::open_workspace(&root).await;
    client.open_file(&model_path).await.unwrap();
    let diags = client.collect_diagnostics(3000).await;
    client.shutdown().await;

    let model_uri = format!("file://{}", model_path.display());
    let model_diags: Vec<_> = diags
        .into_iter()
        .filter(|(uri, _)| *uri == model_uri)
        .flat_map(|(_, ds)| ds)
        .collect();
    assert!(
        has_skeleton_changed(&model_diags),
        "expected maintenance-skeleton-changed for {model_uri}; got {model_diags:?}"
    );
}

/// Writing a deployed-schema snapshot AFTER the workspace is already open,
/// then delivering it via `workspace/didChangeWatchedFiles`, refreshes the
/// diagnostic without an editor restart — the `.smelt/targets/*/schemas/
/// *.json` watch glob this outcome adds.
#[tokio::test]
async fn schema_snapshot_change_refreshes_diagnostics() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    write_fixture(&root);
    // No snapshot yet at workspace-load time — clean.

    let model_path = root.join("models/device_user_counts.sql");
    let mut client = TestClient::open_workspace(&root).await;
    client.open_file(&model_path).await.unwrap();
    let initial_diags = client.collect_diagnostics(2000).await;
    let model_uri = format!("file://{}", model_path.display());
    let initial_model_diags: Vec<_> = initial_diags
        .into_iter()
        .filter(|(uri, _)| *uri == model_uri)
        .flat_map(|(_, ds)| ds)
        .collect();
    assert!(
        !has_skeleton_changed(&initial_model_diags),
        "no snapshot exists yet — must not fire before the watch event: {initial_model_diags:?}"
    );

    // Now write the snapshot and deliver the watch-glob change event.
    write_deployed_schema(&root, &["device_id", "n"]);
    let snapshot_path = root.join(".smelt/targets/dev/schemas/device_user_counts.json");
    client.did_change_watched_file(&snapshot_path, 2).await; // 2 = Changed

    let refreshed_diags = client.collect_diagnostics(3000).await;
    client.shutdown().await;
    let refreshed_model_diags: Vec<_> = refreshed_diags
        .into_iter()
        .filter(|(uri, _)| *uri == model_uri)
        .flat_map(|(_, ds)| ds)
        .collect();
    assert!(
        has_skeleton_changed(&refreshed_model_diags),
        "expected maintenance-skeleton-changed after the watch-glob refresh; got \
         {refreshed_model_diags:?}"
    );
}
