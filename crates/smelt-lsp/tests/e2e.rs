//! End-to-end LSP protocol tests using in-process DuplexStream pairs.
//!
//! These tests exercise the full tower-lsp + Backend stack by sending
//! JSON-RPC messages over async byte streams, catching bugs in the gap
//! between "Salsa queries return correct data" and "the client receives
//! a valid LSP response".

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tower_lsp::{LspService, Server};

use smelt_lsp::Backend;

// ---------------------------------------------------------------------------
// Protocol helpers
// ---------------------------------------------------------------------------

fn encode_message(msg: &Value) -> Vec<u8> {
    let body = serde_json::to_string(msg).unwrap();
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body).into_bytes()
}

async fn read_message(stream: &mut DuplexStream) -> Option<Value> {
    // Read headers until \r\n\r\n
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
// TestWorkspaceDir — filesystem setup
// ---------------------------------------------------------------------------

struct TestWorkspaceDir {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl TestWorkspaceDir {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        std::fs::create_dir_all(path.join("models")).unwrap();
        // Create minimal smelt.yml so project discovery finds this workspace
        std::fs::write(
            path.join("smelt.yml"),
            "name: test\nversion: 1\nmodel_paths:\n  - models\n",
        )
        .unwrap();
        Self { _dir: dir, path }
    }

    fn add_model(&self, name: &str, sql: &str) {
        let file_path = self.path.join("models").join(format!("{}.sql", name));
        std::fs::write(&file_path, sql).unwrap();
    }

    #[allow(dead_code)]
    fn set_sources_yml(&self, content: &str) {
        std::fs::write(self.path.join("sources.yml"), content).unwrap();
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn model_uri(&self, name: &str) -> String {
        let p = self.path.join("models").join(format!("{}.sql", name));
        format!("file://{}", p.display())
    }
}

// ---------------------------------------------------------------------------
// TestClient — DuplexStream-based LSP client
// ---------------------------------------------------------------------------

struct TestClient {
    /// Stream for reading server responses/notifications
    client_rx: DuplexStream,
    /// Stream for writing client requests/notifications
    client_tx: DuplexStream,
    /// Next JSON-RPC request ID
    next_id: i64,
    /// Buffered notifications received while waiting for responses
    notification_buffer: Vec<Value>,
}

impl TestClient {
    async fn new(workspace_dir: &Path) -> Self {
        let (service, socket) = LspService::new(Backend::new);

        // Create two duplex pairs:
        // client_tx_write -> server_stdin_read (client sends to server)
        // server_stdout_write -> client_rx_read (server sends to client)
        let (server_stdin_read, client_tx_write) = tokio::io::duplex(64 * 1024);
        let (client_rx_read, server_stdout_write) = tokio::io::duplex(64 * 1024);

        // Spawn the server
        tokio::spawn(async move {
            Server::new(server_stdin_read, server_stdout_write, socket)
                .serve(service)
                .await;
        });

        let mut client = Self {
            client_rx: client_rx_read,
            client_tx: client_tx_write,
            next_id: 1,
            notification_buffer: Vec::new(),
        };

        // Send initialize request
        let workspace_uri = format!("file://{}", workspace_dir.display());
        let init_result = client
            .send_request(
                "initialize",
                json!({
                    "processId": null,
                    "capabilities": {
                        "workspace": {
                            "workspaceFolders": true
                        }
                    },
                    "workspaceFolders": [{
                        "uri": workspace_uri,
                        "name": "test"
                    }]
                }),
            )
            .await;
        assert!(
            init_result.get("capabilities").is_some(),
            "initialize should return capabilities"
        );

        // Send initialized notification
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

        // Read messages until we get the response with our ID
        loop {
            let response = read_message_timeout(&mut self.client_rx, 5000)
                .await
                .unwrap_or_else(|| {
                    panic!("Timeout waiting for response to {} (id={})", method, id)
                });

            if response.get("id").and_then(|v| v.as_i64()) == Some(id) {
                // This is our response
                if let Some(error) = response.get("error") {
                    panic!("LSP error for {}: {}", method, error);
                }
                return response.get("result").cloned().unwrap_or(Value::Null);
            } else {
                // This is a notification — buffer it
                self.notification_buffer.push(response);
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

    async fn open_file(&mut self, uri: &str, text: &str) {
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
    }

    async fn change_file(&mut self, uri: &str, text: &str, version: i32) {
        self.send_notification(
            "textDocument/didChange",
            json!({
                "textDocument": {
                    "uri": uri,
                    "version": version
                },
                "contentChanges": [{ "text": text }]
            }),
        )
        .await;
    }

    /// Collect all publishDiagnostics notifications, waiting up to timeout_ms.
    /// Returns (uri, diagnostics) pairs.
    async fn collect_diagnostics(
        &mut self,
        timeout_ms: u64,
    ) -> Vec<(String, Vec<lsp_types::Diagnostic>)> {
        // First drain any buffered notifications
        let mut results = Vec::new();
        let buffered = std::mem::take(&mut self.notification_buffer);
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

        // Then read new notifications until timeout
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
            // Other notifications are ignored
        }

        results
    }

    async fn rename(&mut self, uri: &str, line: u32, col: u32, new_name: &str) -> Value {
        self.send_request(
            "textDocument/rename",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": col },
                "newName": new_name
            }),
        )
        .await
    }

    #[allow(dead_code)]
    async fn prepare_rename(&mut self, uri: &str, line: u32, col: u32) -> Value {
        self.send_request(
            "textDocument/prepareRename",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": col }
            }),
        )
        .await
    }

    async fn goto_definition(&mut self, uri: &str, line: u32, col: u32) -> Value {
        self.send_request(
            "textDocument/definition",
            json!({
                "textDocument": { "uri": uri },
                "position": { "line": line, "character": col }
            }),
        )
        .await
    }

    async fn code_actions(
        &mut self,
        uri: &str,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
    ) -> Value {
        self.send_request(
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": uri },
                "range": {
                    "start": { "line": start_line, "character": start_col },
                    "end": { "line": end_line, "character": end_col }
                },
                "context": { "diagnostics": [] }
            }),
        )
        .await
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
        // Read shutdown response (best effort)
        let _ = read_message_timeout(&mut self.client_rx, 1000).await;

        self.send_notification("exit", json!(null)).await;
    }
}

// ---------------------------------------------------------------------------
// Helper: check no overlapping edits
// ---------------------------------------------------------------------------

fn assert_no_overlapping_edits(edit: &Value) {
    // Check both "changes" and "documentChanges" formats
    if let Some(changes) = edit.get("changes").and_then(|c| c.as_object()) {
        for (uri, edits) in changes {
            check_edits_no_overlap(uri, edits.as_array().unwrap());
        }
    }
    if let Some(doc_changes) = edit.get("documentChanges").and_then(|c| c.as_array()) {
        for change in doc_changes {
            if let Some(text_edit) = change.get("edits") {
                let uri = change
                    .pointer("/textDocument/uri")
                    .and_then(|u| u.as_str())
                    .unwrap_or("<unknown>");
                let edits: Vec<&Value> = text_edit
                    .as_array()
                    .unwrap()
                    .iter()
                    // Handle OneOf<TextEdit, AnnotatedTextEdit>
                    .filter(|e| e.get("range").is_some())
                    .collect();
                check_edits_no_overlap_refs(uri, &edits);
            }
        }
    }
}

fn check_edits_no_overlap(uri: &str, edits: &[Value]) {
    let refs: Vec<&Value> = edits.iter().collect();
    check_edits_no_overlap_refs(uri, &refs);
}

fn check_edits_no_overlap_refs(uri: &str, edits: &[&Value]) {
    // Extract (start_line, start_col, end_line, end_col) for each edit
    let mut ranges: Vec<(u32, u32, u32, u32)> = edits
        .iter()
        .filter_map(|e| {
            let range = e.get("range")?;
            let start = range.get("start")?;
            let end = range.get("end")?;
            Some((
                start.get("line")?.as_u64()? as u32,
                start.get("character")?.as_u64()? as u32,
                end.get("line")?.as_u64()? as u32,
                end.get("character")?.as_u64()? as u32,
            ))
        })
        .collect();

    ranges.sort();

    for i in 1..ranges.len() {
        let prev = ranges[i - 1];
        let curr = ranges[i];
        // prev end must be <= curr start
        let prev_end = (prev.2, prev.3);
        let curr_start = (curr.0, curr.1);
        assert!(
            prev_end <= curr_start,
            "Overlapping edits in {}: {:?} overlaps {:?}",
            uri,
            prev,
            curr
        );
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[tokio::test]
async fn test_initialize_and_diagnostics() {
    let ws = TestWorkspaceDir::new();
    ws.add_model("users", "SELECT id, name FROM smelt.models.missing");
    let mut client = TestClient::new(ws.path()).await;

    // Open the file to trigger diagnostics
    let uri = ws.model_uri("users");
    ws.add_model("users", "SELECT id, name FROM smelt.models.missing");
    client
        .open_file(&uri, "SELECT id, name FROM smelt.models.missing")
        .await;

    let diags = client.collect_diagnostics(2000).await;

    // Should have diagnostics for the undefined ref 'missing'
    let model_diags: Vec<_> = diags
        .iter()
        .filter(|(u, d)| u.contains("users") && !d.is_empty())
        .collect();
    assert!(
        !model_diags.is_empty(),
        "Expected diagnostics for undefined ref 'missing', got: {:?}",
        diags
    );

    client.shutdown().await;
}

/// Regression test: CTE rename updates both definition and all references
#[tokio::test]
async fn test_rename_cte() {
    let ws = TestWorkspaceDir::new();
    let sql = "WITH cte AS (SELECT 1 AS id) SELECT id FROM cte";
    ws.add_model("test_cte", sql);
    let mut client = TestClient::new(ws.path()).await;

    let uri = ws.model_uri("test_cte");
    client.open_file(&uri, sql).await;
    // Drain init diagnostics
    client.collect_diagnostics(1000).await;

    // Rename "cte" at the definition site (line 0, col 5 = start of "cte" in "WITH cte AS")
    let edit = client.rename(&uri, 0, 5, "renamed_cte").await;
    assert_no_overlapping_edits(&edit);

    // Should have edits for both the definition and the reference in FROM
    let changes = edit.get("changes").expect("rename should return changes");
    let file_edits = changes.get(&uri).expect("edits for the model file");
    let edits_array = file_edits.as_array().unwrap();
    assert!(
        edits_array.len() >= 2,
        "Expected at least 2 edits (definition + reference), got {}",
        edits_array.len()
    );

    client.shutdown().await;
}

/// Regression test: column rename propagates to upstream model definition (bug #2)
#[tokio::test]
async fn test_rename_column_propagates_upstream() {
    let ws = TestWorkspaceDir::new();
    ws.add_model(
        "events",
        "SELECT id, properties FROM smelt.models.raw_events",
    );
    ws.add_model(
        "event_properties",
        "SELECT e.properties FROM smelt.models.events e",
    );
    ws.add_model("raw_events", "SELECT 1 AS id, 'x' AS properties");
    let mut client = TestClient::new(ws.path()).await;

    let events_uri = ws.model_uri("events");
    let ep_uri = ws.model_uri("event_properties");
    let raw_uri = ws.model_uri("raw_events");
    client
        .open_file(
            &events_uri,
            "SELECT id, properties FROM smelt.models.raw_events",
        )
        .await;
    client
        .open_file(&ep_uri, "SELECT e.properties FROM smelt.models.events e")
        .await;
    client
        .open_file(&raw_uri, "SELECT 1 AS id, 'x' AS properties")
        .await;
    // Drain init diagnostics
    client.collect_diagnostics(1000).await;

    // Rename "properties" from event_properties.sql (col position after "e.")
    // "SELECT e.properties" — "properties" starts at col 9
    let edit = client.rename(&ep_uri, 0, 9, "props").await;
    assert_no_overlapping_edits(&edit);

    // Should have edits in multiple files (at least event_properties + events or raw_events)
    let edit_str = serde_json::to_string_pretty(&edit).unwrap();
    // Check that the edit spans more than one file
    let has_doc_changes = edit.get("documentChanges").is_some();
    let has_multi_file_changes = edit
        .get("changes")
        .and_then(|c| c.as_object())
        .map(|o| o.len() > 1)
        .unwrap_or(false);

    assert!(
        has_doc_changes || has_multi_file_changes,
        "Expected cross-file edits for column rename, got: {}",
        edit_str
    );

    client.shutdown().await;
}

/// Regression test: no stale diagnostics after model rename (bug #3)
///
/// Phase 4: updated to use path form (`smelt.models.*`). The LSP rename
/// operation is not yet implemented for path-form refs, so we simulate
/// the rename manually: open a `new_upstream` file, then update downstream
/// to reference `smelt.models.new_upstream`. The core invariant being
/// tested is that no stale "undefined ref" diagnostic for `new_upstream`
/// remains after the downstream is updated.
#[tokio::test]
async fn test_no_stale_diagnostics_after_model_rename() {
    let ws = TestWorkspaceDir::new();
    ws.add_model("upstream", "SELECT 1 AS id");
    // Phase 4: use path form (smelt.ref() is removed).
    ws.add_model("downstream", "SELECT id FROM smelt.models.upstream");
    let mut client = TestClient::new(ws.path()).await;

    let upstream_uri = ws.model_uri("upstream");
    let downstream_uri = ws.model_uri("downstream");
    client.open_file(&upstream_uri, "SELECT 1 AS id").await;
    client
        .open_file(&downstream_uri, "SELECT id FROM smelt.models.upstream")
        .await;
    // Drain init diagnostics — should have no error-severity diagnostics.
    let init_diags = client.collect_diagnostics(1000).await;
    let errors: Vec<_> = init_diags
        .iter()
        .filter(|(_, d)| {
            d.iter().any(|diag| {
                diag.message.contains("error")
                    || diag.message.contains("removed")
                    || diag.message.contains("undefined")
                    || diag.message.contains("Undefined")
            })
        })
        .collect();
    assert!(
        errors.is_empty(),
        "Expected no initial error diagnostics, got: {:?}",
        errors
    );

    // Simulate the rename: open new_upstream, then update downstream to
    // reference smelt.models.new_upstream.
    let new_upstream_uri = ws.model_uri("new_upstream");
    client.open_file(&new_upstream_uri, "SELECT 1 AS id").await;
    client
        .change_file(
            &downstream_uri,
            "SELECT id FROM smelt.models.new_upstream",
            2,
        )
        .await;

    // Collect diagnostics after the simulated rename
    let post_rename_diags = client.collect_diagnostics(2000).await;

    // There should be NO "undefined model reference" diagnostics for "new_upstream"
    let stale_errors: Vec<_> = post_rename_diags
        .iter()
        .filter(|(_, diags)| {
            diags.iter().any(|d| {
                d.message.contains("undefined") && d.message.to_lowercase().contains("new_upstream")
            })
        })
        .collect();
    assert!(
        stale_errors.is_empty(),
        "Should have no stale diagnostics for 'new_upstream' after rename, got: {:?}",
        stale_errors
    );

    client.shutdown().await;
}

/// Regression test: multiline SELECT rename produces no overlapping edits (bug #1)
#[tokio::test]
async fn test_rename_no_overlapping_edits_multiline() {
    let ws = TestWorkspaceDir::new();
    let sql = "SELECT\n    id,\n    properties\nFROM smelt.models.raw";
    ws.add_model("events", sql);
    ws.add_model("raw", "SELECT 1 AS id, 'x' AS properties");
    let mut client = TestClient::new(ws.path()).await;

    let events_uri = ws.model_uri("events");
    let raw_uri = ws.model_uri("raw");
    client.open_file(&events_uri, sql).await;
    client
        .open_file(&raw_uri, "SELECT 1 AS id, 'x' AS properties")
        .await;
    // Drain init diagnostics
    client.collect_diagnostics(1000).await;

    // Rename "properties" at line 2, col 4
    let edit = client.rename(&events_uri, 2, 4, "props").await;
    assert_no_overlapping_edits(&edit);

    client.shutdown().await;
}

/// Goto-definition for smelt.ref() jumps to the upstream model file
#[tokio::test]
async fn test_goto_definition_ref() {
    let ws = TestWorkspaceDir::new();
    ws.add_model("upstream", "SELECT 1 AS id");
    ws.add_model("downstream", "SELECT id FROM smelt.models.upstream");
    let mut client = TestClient::new(ws.path()).await;

    let upstream_uri = ws.model_uri("upstream");
    let downstream_uri = ws.model_uri("downstream");
    client.open_file(&upstream_uri, "SELECT 1 AS id").await;
    client
        .open_file(&downstream_uri, "SELECT id FROM smelt.models.upstream")
        .await;
    client.collect_diagnostics(1000).await;

    // Goto-definition on 'upstream' inside ref call
    // "SELECT id FROM smelt.models.upstream" — 'upstream' starts at col 26
    let result = client.goto_definition(&downstream_uri, 0, 26).await;

    // Should point to upstream.sql
    let result_str = serde_json::to_string(&result).unwrap();
    assert!(
        result_str.contains("upstream"),
        "goto-definition should point to upstream.sql, got: {}",
        result_str
    );
    // Should be at line 0
    assert!(
        result_str.contains("\"line\":0"),
        "goto-definition should point to line 0, got: {}",
        result_str
    );

    client.shutdown().await;
}

/// Goto-definition for a column traces through to the upstream definition
#[tokio::test]
async fn test_goto_definition_column() {
    let ws = TestWorkspaceDir::new();
    ws.add_model("upstream", "SELECT 1 AS id, 'hello' AS name");
    ws.add_model("downstream", "SELECT u.id FROM smelt.models.upstream u");
    let mut client = TestClient::new(ws.path()).await;

    let upstream_uri = ws.model_uri("upstream");
    let downstream_uri = ws.model_uri("downstream");
    client
        .open_file(&upstream_uri, "SELECT 1 AS id, 'hello' AS name")
        .await;
    client
        .open_file(&downstream_uri, "SELECT u.id FROM smelt.models.upstream u")
        .await;
    client.collect_diagnostics(1000).await;

    // Goto-definition on "id" in "u.id" — "id" starts at col 9
    let result = client.goto_definition(&downstream_uri, 0, 9).await;

    // Should point to upstream.sql where "id" is defined
    let result_str = serde_json::to_string(&result).unwrap();
    assert!(
        result_str.contains("upstream"),
        "goto-definition should point to upstream.sql, got: {}",
        result_str
    );

    client.shutdown().await;
}

/// Code action: create missing model offered for undefined ref
#[tokio::test]
async fn test_code_action_create_missing_model() {
    let ws = TestWorkspaceDir::new();
    ws.add_model("model", "SELECT id FROM smelt.models.nonexistent");
    let mut client = TestClient::new(ws.path()).await;

    let uri = ws.model_uri("model");
    client
        .open_file(&uri, "SELECT id FROM smelt.models.nonexistent")
        .await;
    let diags = client.collect_diagnostics(1000).await;

    // Should have an undefined ref diagnostic
    let model_diags: Vec<_> = diags
        .iter()
        .filter(|(u, d)| u.contains("model") && !d.is_empty())
        .collect();
    assert!(!model_diags.is_empty(), "Expected undefined ref diagnostic");

    // Request code actions at the diagnostic range
    // smelt.models.nonexistent — the ref call starts around col 15
    let actions = client.code_actions(&uri, 0, 15, 0, 40).await;
    let actions_str = serde_json::to_string(&actions).unwrap();

    // Should offer a "Create" code action
    assert!(
        actions_str.to_lowercase().contains("create"),
        "Expected a 'Create' code action, got: {}",
        actions_str
    );

    client.shutdown().await;
}

/// Diagnostics clear after fixing the error (adding missing model)
#[tokio::test]
async fn test_diagnostics_clear_after_fix() {
    let ws = TestWorkspaceDir::new();
    ws.add_model("model", "SELECT id FROM smelt.models.missing");
    let mut client = TestClient::new(ws.path()).await;

    let model_uri = ws.model_uri("model");
    client
        .open_file(&model_uri, "SELECT id FROM smelt.models.missing")
        .await;

    // Collect initial diagnostics — should show undefined ref
    let diags = client.collect_diagnostics(1000).await;
    let has_error = diags
        .iter()
        .any(|(u, d)| u.contains("model") && !d.is_empty());
    assert!(has_error, "Expected undefined ref diagnostic initially");

    // Fix: add the missing model
    let missing_uri = ws.model_uri("missing");
    client.open_file(&missing_uri, "SELECT 1 AS id").await;

    // Trigger re-diagnosis by sending didChange for model.sql (same content)
    client
        .change_file(&model_uri, "SELECT id FROM smelt.models.missing", 2)
        .await;

    // Collect diagnostics after fix
    let post_diags = client.collect_diagnostics(2000).await;

    // Find the LAST diagnostic notification for model.sql (latest wins)
    let model_diags: Vec<_> = post_diags
        .iter()
        .filter(|(u, _)| u.contains("model.sql"))
        .collect();
    if let Some((_, diags)) = model_diags.last() {
        let undefined_refs: Vec<_> = diags
            .iter()
            .filter(|d| d.message.contains("undefined") || d.message.contains("Undefined"))
            .collect();
        assert!(
            undefined_refs.is_empty(),
            "Expected no undefined ref diagnostics after fix, got: {:?}",
            undefined_refs
        );
    }

    client.shutdown().await;
}
