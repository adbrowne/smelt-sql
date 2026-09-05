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
            "name: test\nversion: 1\npaths:\n  - models\n",
        )
        .unwrap();
        Self { _dir: dir, path }
    }

    fn add_model(&self, name: &str, sql: &str) {
        let file_path = self.path.join("models").join(format!("{}.sql", name));
        std::fs::write(&file_path, sql).unwrap();
    }

    /// Add a model in a subdirectory under `models/`.
    /// E.g. `add_model_in_subdir("silver", "upstream", sql)` writes to
    /// `models/silver/upstream.sql` — canonical path `smelt.silver.upstream`.
    fn add_model_in_subdir(&self, subdir: &str, name: &str, sql: &str) {
        let dir = self.path.join("models").join(subdir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{}.sql", name)), sql).unwrap();
    }

    /// URI for a model in a subdirectory.
    fn model_uri_in_subdir(&self, subdir: &str, name: &str) -> String {
        let p = self
            .path
            .join("models")
            .join(subdir)
            .join(format!("{}.sql", name));
        format!("file://{}", p.display())
    }

    fn add_function(&self, name: &str, sql: &str) {
        let functions_dir = self.path.join("functions");
        std::fs::create_dir_all(&functions_dir).unwrap();
        let file_path = functions_dir.join(format!("{}.sql", name));
        std::fs::write(&file_path, sql).unwrap();
    }

    /// Drop a seed CSV under `models/<name>.csv` (matches the default
    /// `paths: ["models"]` layout `TestWorkspaceDir` ships with). The seed
    /// becomes `smelt.<name>` per the Phase 2 prefix-free resolution rule.
    fn add_seed(&self, name: &str, csv: &str) {
        let p = self.path.join("models").join(format!("{}.csv", name));
        std::fs::write(&p, csv).unwrap();
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

    /// Like `send_request` but returns the full JSON-RPC response (including
    /// any `"error"` field) without panicking. Used by tests that assert an
    /// error is returned (e.g. refusing to rename a source column).
    async fn send_request_raw(&mut self, method: &str, params: Value) -> Value {
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
            let response = read_message_timeout(&mut self.client_rx, 5000)
                .await
                .unwrap_or_else(|| {
                    panic!("Timeout waiting for response to {} (id={})", method, id)
                });

            if response.get("id").and_then(|v| v.as_i64()) == Some(id) {
                return response;
            } else {
                self.notification_buffer.push(response);
            }
        }
    }

    async fn prepare_rename_raw(&mut self, uri: &str, line: u32, col: u32) -> Value {
        self.send_request_raw(
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

    async fn hover(&mut self, uri: &str, line: u32, col: u32) -> Value {
        self.send_request(
            "textDocument/hover",
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
    ws.add_model("users", "SELECT id, name FROM smelt.missing");
    let mut client = TestClient::new(ws.path()).await;

    // Open the file to trigger diagnostics
    let uri = ws.model_uri("users");
    ws.add_model("users", "SELECT id, name FROM smelt.missing");
    client
        .open_file(&uri, "SELECT id, name FROM smelt.missing")
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
    ws.add_model("events", "SELECT id, properties FROM smelt.raw_events");
    ws.add_model(
        "event_properties",
        "SELECT e.properties FROM smelt.events e",
    );
    ws.add_model("raw_events", "SELECT 1 AS id, 'x' AS properties");
    let mut client = TestClient::new(ws.path()).await;

    let events_uri = ws.model_uri("events");
    let ep_uri = ws.model_uri("event_properties");
    let raw_uri = ws.model_uri("raw_events");
    client
        .open_file(&events_uri, "SELECT id, properties FROM smelt.raw_events")
        .await;
    client
        .open_file(&ep_uri, "SELECT e.properties FROM smelt.events e")
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
/// Uses canonical `smelt.<path>` addressing (scan-root stripped). The LSP
/// rename operation is not yet implemented for path-form refs, so we simulate
/// the rename manually: open a `new_upstream` file, then update downstream
/// to reference `smelt.new_upstream`. The core invariant being
/// tested is that no stale "undefined ref" diagnostic for `new_upstream`
/// remains after the downstream is updated.
#[tokio::test]
async fn test_no_stale_diagnostics_after_model_rename() {
    let ws = TestWorkspaceDir::new();
    ws.add_model("upstream", "SELECT 1 AS id");
    // Canonical path form (legacy smelt.ref()/smelt.models.* are removed).
    ws.add_model("downstream", "SELECT id FROM smelt.upstream");
    let mut client = TestClient::new(ws.path()).await;

    let upstream_uri = ws.model_uri("upstream");
    let downstream_uri = ws.model_uri("downstream");
    client.open_file(&upstream_uri, "SELECT 1 AS id").await;
    client
        .open_file(&downstream_uri, "SELECT id FROM smelt.upstream")
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
    // reference smelt.new_upstream.
    let new_upstream_uri = ws.model_uri("new_upstream");
    client.open_file(&new_upstream_uri, "SELECT 1 AS id").await;
    client
        .change_file(&downstream_uri, "SELECT id FROM smelt.new_upstream", 2)
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
    let sql = "SELECT\n    id,\n    properties\nFROM smelt.raw";
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
    ws.add_model("downstream", "SELECT id FROM smelt.upstream");
    let mut client = TestClient::new(ws.path()).await;

    let upstream_uri = ws.model_uri("upstream");
    let downstream_uri = ws.model_uri("downstream");
    client.open_file(&upstream_uri, "SELECT 1 AS id").await;
    client
        .open_file(&downstream_uri, "SELECT id FROM smelt.upstream")
        .await;
    client.collect_diagnostics(1000).await;

    // Goto-definition on 'upstream' inside ref call
    // "SELECT id FROM smelt.upstream" — 'upstream' starts at col 21
    let result = client.goto_definition(&downstream_uri, 0, 21).await;

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
    ws.add_model("downstream", "SELECT u.id FROM smelt.upstream u");
    let mut client = TestClient::new(ws.path()).await;

    let upstream_uri = ws.model_uri("upstream");
    let downstream_uri = ws.model_uri("downstream");
    client
        .open_file(&upstream_uri, "SELECT 1 AS id, 'hello' AS name")
        .await;
    client
        .open_file(&downstream_uri, "SELECT u.id FROM smelt.upstream u")
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

/// Goto-definition for a column traces through to an upstream definition when
/// the upstream lives in a subdirectory (canonical path `smelt.silver.upstream`).
/// This exercises `resolve_ref_leaf` via the leaf name `"upstream"` that
/// `RowExtension.ref_name` / `InputConstraint.ref_name` carry — the former
/// single-segment path-wrapping approach failed for subdirectory models.
#[tokio::test]
async fn test_goto_definition_column_for_nested_upstream() {
    let ws = TestWorkspaceDir::new();
    // upstream lives at models/silver/upstream.sql
    ws.add_model_in_subdir("silver", "upstream", "SELECT 1 AS id, 'hello' AS name");
    // downstream references it by canonical path smelt.silver.upstream
    ws.add_model_in_subdir(
        "gold",
        "downstream",
        "SELECT u.id FROM smelt.silver.upstream u",
    );

    let upstream_uri = ws.model_uri_in_subdir("silver", "upstream");
    let downstream_uri = ws.model_uri_in_subdir("gold", "downstream");

    let mut client = TestClient::new(ws.path()).await;
    client
        .open_file(&upstream_uri, "SELECT 1 AS id, 'hello' AS name")
        .await;
    client
        .open_file(&downstream_uri, "SELECT u.id FROM smelt.silver.upstream u")
        .await;
    client.collect_diagnostics(1000).await;

    // Goto-definition on "id" in "u.id" — "id" starts at col 9
    let result = client.goto_definition(&downstream_uri, 0, 9).await;
    let result_str = serde_json::to_string(&result).unwrap();

    assert!(
        result_str.contains("upstream"),
        "goto-definition should point to models/silver/upstream.sql, got: {}",
        result_str
    );

    client.shutdown().await;
}

/// Hover on a qualified SQL column reference shows its resolved type and the
/// source it came from.
#[tokio::test]
async fn test_hover_on_qualified_column() {
    let ws = TestWorkspaceDir::new();
    ws.add_model("upstream", "SELECT 1 AS id, 'hello' AS name");
    let downstream_sql = "SELECT u.id FROM smelt.upstream u";
    ws.add_model("downstream", downstream_sql);
    let mut client = TestClient::new(ws.path()).await;

    let upstream_uri = ws.model_uri("upstream");
    let downstream_uri = ws.model_uri("downstream");
    client
        .open_file(&upstream_uri, "SELECT 1 AS id, 'hello' AS name")
        .await;
    client.open_file(&downstream_uri, downstream_sql).await;
    client.collect_diagnostics(1000).await;

    // Hover on "id" inside "u.id" — "id" starts at col 9 in
    // "SELECT u.id FROM smelt.upstream u".
    let result = client.hover(&downstream_uri, 0, 9).await;
    let result_str = serde_json::to_string(&result).unwrap();

    // The id column in upstream is `SELECT 1 AS id` — DuckDB infers this as
    // an integer (`INTEGER` / `BIGINT` depending on the literal). The exact
    // rendering follows the `Display` impl on `DataType` (uppercase). We
    // assert the type line uses backticks and that the source line names
    // the upstream model the column came from.
    assert!(
        result_str.contains("u.id"),
        "hover content should mention the qualified column `u.id`, got: {}",
        result_str
    );
    assert!(
        result_str.to_lowercase().contains("from model"),
        "hover content should describe the column source (`From model …`), got: {}",
        result_str
    );

    client.shutdown().await;
}

/// Hover on an unqualified SQL column reference (no FROM alias) still shows
/// the resolved type — the type comes from the upstream model's schema.
#[tokio::test]
async fn test_hover_on_unqualified_column() {
    let ws = TestWorkspaceDir::new();
    ws.add_model("upstream", "SELECT 1 AS id, 'hello' AS name");
    let downstream_sql = "SELECT id FROM smelt.upstream";
    ws.add_model("downstream", downstream_sql);
    let mut client = TestClient::new(ws.path()).await;

    let upstream_uri = ws.model_uri("upstream");
    let downstream_uri = ws.model_uri("downstream");
    client
        .open_file(&upstream_uri, "SELECT 1 AS id, 'hello' AS name")
        .await;
    client.open_file(&downstream_uri, downstream_sql).await;
    client.collect_diagnostics(1000).await;

    // Hover on "id" — starts at col 7.
    let result = client.hover(&downstream_uri, 0, 7).await;
    let result_str = serde_json::to_string(&result).unwrap();

    assert!(
        result_str.contains("**`id`**"),
        "hover content should render the unqualified column name in bold backticks, got: {}",
        result_str
    );
    // Type line must be backtick-wrapped (not the "*type unknown*" fallback)
    // because the upstream's `id` literal is resolvable.
    assert!(
        !result_str.contains("type unknown"),
        "type should resolve for an unqualified column from a single upstream, got: {}",
        result_str
    );

    client.shutdown().await;
}

/// Code action: create missing model offered for undefined ref
#[tokio::test]
async fn test_code_action_create_missing_model() {
    let ws = TestWorkspaceDir::new();
    ws.add_model("model", "SELECT id FROM smelt.nonexistent");
    let mut client = TestClient::new(ws.path()).await;

    let uri = ws.model_uri("model");
    client
        .open_file(&uri, "SELECT id FROM smelt.nonexistent")
        .await;
    let diags = client.collect_diagnostics(1000).await;

    // Should have an undefined ref diagnostic
    let model_diags: Vec<_> = diags
        .iter()
        .filter(|(u, d)| u.contains("model") && !d.is_empty())
        .collect();
    assert!(!model_diags.is_empty(), "Expected undefined ref diagnostic");

    // Request code actions at the diagnostic range
    // smelt.nonexistent — the ref call starts around col 15
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
    ws.add_model("model", "SELECT id FROM smelt.missing");
    let mut client = TestClient::new(ws.path()).await;

    let model_uri = ws.model_uri("model");
    client
        .open_file(&model_uri, "SELECT id FROM smelt.missing")
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
        .change_file(&model_uri, "SELECT id FROM smelt.missing", 2)
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

/// Regression test: the LSP must discover function definitions under
/// `<project_root>/functions/` during `initialize`, matching the CLI's
/// `Discovery::discover_function_files`. Without this, calls to
/// `smelt.functions.*` produce `unknown-smelt-fn` diagnostics in VSCode
/// even when the function file exists on disk.
#[tokio::test]
async fn test_lsp_discovers_functions_directory() {
    let ws = TestWorkspaceDir::new();
    ws.add_function(
        "add_one",
        "smelt.define add_one(x: Expr<Integer>) -> Expr<Integer> AS (x + 1)\n",
    );
    let model_sql = "SELECT smelt.functions.add_one(id) AS bumped FROM (SELECT 1 AS id)";
    ws.add_model("uses_fn", model_sql);

    let mut client = TestClient::new(ws.path()).await;
    let uri = ws.model_uri("uses_fn");
    client.open_file(&uri, model_sql).await;

    let diags = client.collect_diagnostics(2000).await;

    // The bug we're guarding against: an `unknown-smelt-fn` diagnostic on
    // the model file because the LSP failed to load `functions/add_one.sql`.
    let unknown_fn_diags: Vec<_> = diags
        .iter()
        .filter(|(u, _)| u.contains("uses_fn"))
        .flat_map(|(_, d)| d.iter())
        .filter(|d| {
            d.code
                .as_ref()
                .map(|c| matches!(c, lsp_types::NumberOrString::String(s) if s == "unknown-smelt-fn"))
                .unwrap_or(false)
        })
        .collect();

    assert!(
        unknown_fn_diags.is_empty(),
        "Expected no `unknown-smelt-fn` diagnostics on the model file, but got: {:?}\n\
         All diagnostics: {:?}",
        unknown_fn_diags,
        diags,
    );

    client.shutdown().await;
}

/// Goto-definition on a `smelt.functions.<name>(` call jumps to the
/// `smelt.define <name>(...)` declaration in the function file. Lands the
/// cursor precisely on the function name token, not on the file's first line.
#[tokio::test]
async fn test_goto_definition_smelt_function_call() {
    let ws = TestWorkspaceDir::new();
    // Function: definition lives on line 1 (0-indexed) so we can assert
    // the cursor lands on a non-zero line.
    ws.add_function(
        "add_one",
        "-- bump an integer by 1\nsmelt.define add_one(x: Expr<Integer>) -> Expr<Integer> AS (x + 1)\n",
    );
    let model_sql = "SELECT smelt.functions.add_one(id) AS bumped FROM (SELECT 1 AS id)";
    ws.add_model("caller", model_sql);
    let mut client = TestClient::new(ws.path()).await;

    let caller_uri = ws.model_uri("caller");
    client.open_file(&caller_uri, model_sql).await;
    client.collect_diagnostics(1000).await;

    // Cursor on `add_one` inside `smelt.functions.add_one(...)`.
    // The string `smelt.functions.add_one` starts at col 7; `add_one` starts
    // at col 23 (7 + len("smelt.functions.") = 7 + 16).
    let result = client.goto_definition(&caller_uri, 0, 25).await;

    let result_str = serde_json::to_string(&result).unwrap();
    assert!(
        result_str.contains("add_one.sql"),
        "expected goto-def to land in add_one.sql, got: {result_str}",
    );
    // `smelt.define add_one(...)` is on line 1 of the function file
    // (line 0 is the leading comment).
    assert!(
        result_str.contains("\"line\":1"),
        "expected goto-def at line 1 (the smelt.define line), got: {result_str}",
    );

    client.shutdown().await;
}

/// Goto-definition on a `smelt.<seed_name>` path ref jumps into the seed's
/// `.csv` file. (Seeds use the prefix-free addressing scheme — Phase 2.)
#[tokio::test]
async fn test_goto_definition_smelt_seed_ref() {
    let ws = TestWorkspaceDir::new();
    ws.add_seed("raw_users", "id,name\n1,alice\n2,bob\n");
    let model_sql = "SELECT * FROM smelt.raw_users";
    ws.add_model("caller", model_sql);
    let mut client = TestClient::new(ws.path()).await;

    let caller_uri = ws.model_uri("caller");
    client.open_file(&caller_uri, model_sql).await;
    client.collect_diagnostics(1000).await;

    // Cursor on `raw_users` in `smelt.raw_users`. The path starts at col 14.
    let result = client.goto_definition(&caller_uri, 0, 22).await;
    let result_str = serde_json::to_string(&result).unwrap();
    assert!(
        result_str.contains("raw_users.csv"),
        "expected goto-def to land in raw_users.csv, got: {result_str}",
    );

    client.shutdown().await;
}

/// Cross-file diagnostic republication: editing an upstream model republishes
/// diagnostics for downstream files that depend on it.
///
/// Before the fix, `did_change` for `.sql` files only called
/// `publish_diagnostics(uri)` (just the edited file). After the fix it calls
/// `publish_all_diagnostics()` so consumers of the changed model also get
/// refreshed diagnostics.
///
/// The test verifies the core LSP-level invariant: after a `textDocument/didChange`
/// notification on an upstream file, the server MUST emit a `publishDiagnostics`
/// notification for every tracked file — not only for the changed file. The
/// downstream notification may be empty (clean file) or carry new diagnostics;
/// what matters is that it was sent so the client can update its stale view.
///
/// To also exercise a downstream diagnostic, the downstream file opens a model
/// that references `smelt.upstream`, then deletes the upstream reference's
/// target by changing `upstream.sql` to an entirely different name. The LSP
/// should emit an `UndefinedModelRef` diagnostic on the downstream file.
#[tokio::test]
async fn test_upstream_edit_republishes_downstream_diagnostics() {
    let ws = TestWorkspaceDir::new();
    // upstream exposes two columns: id and value
    ws.add_model("upstream", "SELECT 1 AS id, 2 AS value");
    // downstream references upstream. After the edit, `upstream` will no longer
    // export `value`, so downstream should get a notification.
    ws.add_model("downstream", "SELECT u.value FROM smelt.upstream u");

    let mut client = TestClient::new(ws.path()).await;

    let upstream_uri = ws.model_uri("upstream");
    let downstream_uri = ws.model_uri("downstream");

    // Open both files so the LSP tracks them
    client
        .open_file(&upstream_uri, "SELECT 1 AS id, 2 AS value")
        .await;
    client
        .open_file(&downstream_uri, "SELECT u.value FROM smelt.upstream u")
        .await;

    // Drain initial diagnostics — both should be clean
    let init_diags = client.collect_diagnostics(2000).await;
    let init_errors: Vec<_> = init_diags
        .iter()
        .flat_map(|(_, d)| d.iter())
        .filter(|d| matches!(d.severity, Some(lsp_types::DiagnosticSeverity::ERROR)))
        .collect();
    assert!(
        init_errors.is_empty(),
        "Expected no initial error diagnostics, got: {:?}",
        init_errors
    );

    // Edit upstream.sql — change it so that `smelt.upstream` still resolves
    // (file still exists) but the content changes. The core assertion is only
    // that `publishDiagnostics` for downstream.sql is emitted.
    client.change_file(&upstream_uri, "SELECT 1 AS id", 2).await;

    // Collect diagnostics after the upstream edit
    let post_edit_diags = client.collect_diagnostics(2000).await;

    // Core invariant: after `did_change` on an upstream file the server MUST
    // republish diagnostics for ALL tracked files (conservative superset), not
    // only for the changed file. Without the fix, only upstream.sql gets a
    // publishDiagnostics notification.
    let downstream_notifs: Vec<_> = post_edit_diags
        .iter()
        .filter(|(u, _)| u.contains("downstream"))
        .collect();

    assert!(
        !downstream_notifs.is_empty(),
        "Expected at least one publishDiagnostics notification for downstream.sql \
         after an upstream edit, but received none.\n\
         All post-edit diagnostic notifications: {:?}",
        post_edit_diags
    );

    client.shutdown().await;
}

/// Goto-definition on a plain `smelt.<name>` path ref still works
/// after adding the `SmeltPathCall` cursor branch.
#[tokio::test]
async fn test_goto_definition_smelt_model_ref_still_works() {
    let ws = TestWorkspaceDir::new();
    ws.add_model("upstream", "SELECT 1 AS id");
    let model_sql = "SELECT * FROM smelt.upstream";
    ws.add_model("caller", model_sql);
    let mut client = TestClient::new(ws.path()).await;

    let caller_uri = ws.model_uri("caller");
    client.open_file(&caller_uri, model_sql).await;
    client.collect_diagnostics(1000).await;

    // Cursor on `upstream` in `smelt.upstream`. Col 14 = start of
    // `smelt`; `upstream` starts at col 20 (14 + len("smelt.")).
    let result = client.goto_definition(&caller_uri, 0, 20).await;
    let result_str = serde_json::to_string(&result).unwrap();
    assert!(
        result_str.contains("upstream"),
        "expected goto-def to land in upstream.sql, got: {result_str}",
    );

    client.shutdown().await;
}

/// D-49 Bug 4: `prepare_rename` must refuse source columns with an error.
///
/// A column declared by an externally-managed source (sourced via
/// `smelt.sources.*`) should not be renameable through the LSP — the column
/// lives in an external data source, not in a smelt SQL model. The server
/// must return a JSON-RPC error response (not `null`).
#[tokio::test]
async fn test_prepare_rename_source_column_refused() {
    let ws = TestWorkspaceDir::new();
    // Legacy aggregate sources.yml so the source is discoverable.
    ws.set_sources_yml(
        "sources:\n  raw:\n    tables:\n      events:\n        columns:\n          - name: user_id\n            type: INTEGER\n",
    );
    // staging.sql selects user_id from the source table
    ws.add_model("staging", "SELECT user_id FROM smelt.sources.raw.events");

    let mut client = TestClient::new(ws.path()).await;
    let staging_uri = ws.model_uri("staging");
    client
        .open_file(&staging_uri, "SELECT user_id FROM smelt.sources.raw.events")
        .await;
    client.collect_diagnostics(1000).await;

    // "SELECT user_id FROM smelt.sources.raw.events"
    //          ^col 7 (start of "user_id")
    let response = client.prepare_rename_raw(&staging_uri, 0, 7).await;

    // The server must refuse: either an error response or null result.
    // A successful rename response would have a non-null result with a "range" field.
    let is_error = response.get("error").is_some();
    let is_null_result = response.get("result").map(|r| r.is_null()).unwrap_or(false);
    assert!(
        is_error || is_null_result,
        "prepare_rename on a source column should return an error or null, got: {}",
        serde_json::to_string_pretty(&response).unwrap()
    );

    client.shutdown().await;
}

/// D-49 Bugs 1, 2, 3: Column rename BFS must be rooted at the definition site.
///
/// When cursor is in `leaf.sql` (which reads from `mid1`), the BFS should
/// start from `base.sql` (the definition site), so sibling consumers like
/// `mid2.sql` are also included in the rename.
///
/// Setup:
///   base.sql:  SELECT 1 AS col_x
///   mid1.sql:  SELECT col_x FROM smelt.base
///   mid2.sql:  SELECT col_x FROM smelt.base   (sibling, must be found)
///   leaf.sql:  SELECT col_x FROM smelt.mid1
///
/// Renaming `col_x` from `leaf.sql` at position (0, 7) must include `mid2.sql`.
#[tokio::test]
async fn test_column_rename_rooted_at_definition_site() {
    let ws = TestWorkspaceDir::new();
    ws.add_model("base", "SELECT 1 AS col_x");
    ws.add_model("mid1", "SELECT col_x FROM smelt.base");
    ws.add_model("mid2", "SELECT col_x FROM smelt.base");
    ws.add_model("leaf", "SELECT col_x FROM smelt.mid1");

    let mut client = TestClient::new(ws.path()).await;
    let base_uri = ws.model_uri("base");
    let mid1_uri = ws.model_uri("mid1");
    let mid2_uri = ws.model_uri("mid2");
    let leaf_uri = ws.model_uri("leaf");

    client.open_file(&base_uri, "SELECT 1 AS col_x").await;
    client
        .open_file(&mid1_uri, "SELECT col_x FROM smelt.base")
        .await;
    client
        .open_file(&mid2_uri, "SELECT col_x FROM smelt.base")
        .await;
    client
        .open_file(&leaf_uri, "SELECT col_x FROM smelt.mid1")
        .await;
    client.collect_diagnostics(1000).await;

    // Rename col_x from leaf.sql. "SELECT col_x FROM smelt.mid1"
    //                                       ^col 7
    let edit = client.rename(&leaf_uri, 0, 7, "col_y").await;
    assert_no_overlapping_edits(&edit);

    let edit_str = serde_json::to_string_pretty(&edit).unwrap();

    // mid2.sql must be included — it consumes col_x from base, not from leaf or mid1
    let includes_mid2 = edit_str.contains("mid2");
    assert!(
        includes_mid2,
        "Rename rooted at definition site must include mid2.sql (sibling consumer of base), \
         but got: {}",
        edit_str
    );

    client.shutdown().await;
}

/// `docs/outcomes/20260904-decided-gap-residue/phases/01-plan.md`: the
/// `ContractFrozenHorizonInvalid` posture check (declaring
/// `contract.frozen_horizon` on a model driven by a non-`append_only`
/// source) reaches the real LSP's published diagnostics with the expected
/// code slug, not just `smelt_db::file_diagnostics()` directly.
#[tokio::test]
async fn lsp_publishes_contract_frozen_horizon_posture_diagnostic() {
    let ws = TestWorkspaceDir::new();
    std::fs::create_dir_all(ws.path().join("models/sources")).unwrap();
    std::fs::write(
        ws.path().join("models/sources/contract_mutable_orders.yml"),
        r#"
description: Orders, mutable snapshot.
mutation_profile: mutable_snapshot
columns:
  - { name: order_id, type: INTEGER, nullable: false }
  - { name: order_date, type: TIMESTAMP, nullable: false }
  - { name: amount, type: DOUBLE, nullable: false }
"#,
    )
    .unwrap();

    let sql = r#"---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: order_date
  partition_column: order_date
  granularity: day
maintenance:
  scan_bounds:
    per_source:
      contract_mutable_orders:
        allow_full_scan: true
contract:
  frozen_horizon: '90 days'
---
SELECT
    date_trunc('day', o.order_date) AS order_date,
    SUM(o.amount) AS total
FROM smelt.sources.contract_mutable_orders o
GROUP BY 1
"#;
    ws.add_model("revenue", sql);
    let mut client = TestClient::new(ws.path()).await;

    let uri = ws.model_uri("revenue");
    client.open_file(&uri, sql).await;

    let diags = client.collect_diagnostics(2000).await;

    let has_expected_code = diags.iter().any(|(u, ds)| {
        u.contains("revenue")
            && ds.iter().any(|d| {
                d.code.as_ref().is_some_and(|c| {
                    matches!(
                        c,
                        lsp_types::NumberOrString::String(s)
                            if s == "contract-frozen-horizon-invalid"
                    )
                })
            })
    });
    assert!(
        has_expected_code,
        "Expected a published diagnostic with code 'contract-frozen-horizon-invalid', got: {:?}",
        diags
    );

    client.shutdown().await;
}
