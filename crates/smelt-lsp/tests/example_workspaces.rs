//! LSP-level diagnostic safety net for every non-broken example workspace.
//!
//! This is Phase 1 of the workspace-loader unification plan: it drives the
//! REAL `Backend` (via tower-lsp + DuplexStream, same as `e2e.rs`) against
//! each `examples/<name>/` and asserts no diagnostics. It complements
//! `crates/smelt-cli/tests/example_diagnostics.rs`, which runs Salsa
//! queries directly and therefore misses bugs that only surface through the
//! LSP startup path (asymmetric discovery, missing loader-file registration,
//! etc.).
//!
//! See `docs/specs/architecture.md` → "Workspace loading parity rule
//! (CLI ↔ LSP)" for why this safety net exists.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tower_lsp::{LspService, Server};

use smelt_lsp::Backend;

// ---------------------------------------------------------------------------
// Protocol helpers (kept self-contained — `e2e.rs`'s helpers are private to
// that file, and duplicating ~80 lines is cheaper than wrestling with the
// crate's test-module layout).
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
            let response = read_message_timeout(&mut self.client_rx, 5000)
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
// Workspace walker — picks every `.sql` under `config.paths` and `functions/`
// so we don't depend on hardcoded subdirectory names per workspace.
// ---------------------------------------------------------------------------

fn workspace_sql_files(workspace_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    // `paths` from smelt.yml (default ["models"]).
    let paths = smelt_core::Config::load(workspace_dir)
        .map(|c| c.paths)
        .unwrap_or_else(|_| vec!["models".to_string()]);
    for rel in paths {
        let dir = workspace_dir.join(&rel);
        if !dir.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("sql") {
                files.push(p.to_path_buf());
            }
        }
    }

    // `functions/` (hardcoded — see smelt_core::discover_function_file_paths).
    for p in smelt_core::discover_function_file_paths(workspace_dir) {
        files.push(p);
    }

    files
}

fn examples_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent() // crates/
        .and_then(|p| p.parent()) // repo root
        .expect("repo root")
        .join("examples")
}

/// Boot the LSP against `examples/<name>/`, open every `.sql` file under
/// `config.paths` and `functions/`, collect all diagnostics, and assert
/// none. Failure messages include file path, code, and message for every
/// diagnostic so the diff against `Missing required argument` (or whatever
/// surfaces) is immediately actionable.
async fn assert_example_workspace_clean(name: &str) {
    let workspace = examples_root().join(name);
    assert!(
        workspace.exists(),
        "example workspace not found: {}",
        workspace.display()
    );

    let files = workspace_sql_files(&workspace);
    assert!(
        !files.is_empty(),
        "no .sql files found in {}; example layout may have changed",
        workspace.display()
    );

    let mut client = TestClient::open_workspace(&workspace).await;
    for file in &files {
        // Some example workspaces may have non-UTF8 or read-restricted files;
        // skip them with a noisy print rather than failing the whole test.
        if let Err(e) = client.open_file(file).await {
            eprintln!("skipping {}: {}", file.display(), e);
        }
    }

    // 3s drain gives the LSP time to publish diagnostics for every opened
    // file. Increase only if the test becomes flaky on slow machines.
    let diags = client.collect_diagnostics(3000).await;
    client.shutdown().await;

    // Group by uri keeping LATEST (subsequent publishes supersede earlier
    // ones for the same file).
    let mut latest: std::collections::HashMap<String, Vec<lsp_types::Diagnostic>> =
        std::collections::HashMap::new();
    for (uri, ds) in diags {
        latest.insert(uri, ds);
    }
    // Filter: any non-empty list is a real diagnostic.
    let dirty: Vec<(String, Vec<lsp_types::Diagnostic>)> = latest
        .into_iter()
        .filter(|(_, ds)| !ds.is_empty())
        .collect();

    if !dirty.is_empty() {
        let mut report = format!(
            "Expected zero diagnostics for examples/{}, but got {} file(s) with diagnostics:\n",
            name,
            dirty.len()
        );
        for (uri, ds) in &dirty {
            report.push_str(&format!("  {uri}\n"));
            for d in ds {
                let code = d
                    .code
                    .as_ref()
                    .map(|c| match c {
                        lsp_types::NumberOrString::Number(n) => n.to_string(),
                        lsp_types::NumberOrString::String(s) => s.clone(),
                    })
                    .unwrap_or_else(|| "?".to_string());
                report.push_str(&format!(
                    "    {}:{} [{}] {}\n",
                    d.range.start.line + 1,
                    d.range.start.character + 1,
                    code,
                    d.message
                ));
            }
        }
        panic!("{report}");
    }
}

// ---------------------------------------------------------------------------
// Per-example tests. One #[tokio::test] per workspace mirrors the structure
// of `crates/smelt-cli/tests/example_diagnostics.rs`.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn web_analytics() {
    assert_example_workspace_clean("web_analytics").await;
}

#[tokio::test]
async fn timeseries() {
    assert_example_workspace_clean("timeseries").await;
}

#[tokio::test]
async fn retail_analytics() {
    assert_example_workspace_clean("retail_analytics").await;
}

#[tokio::test]
async fn test_workspace() {
    assert_example_workspace_clean("test_workspace").await;
}

#[tokio::test]
async fn ephemeral_demo() {
    assert_example_workspace_clean("ephemeral_demo").await;
}

#[tokio::test]
async fn multi_engine() {
    assert_example_workspace_clean("multi_engine").await;
}

#[tokio::test]
async fn ecommerce() {
    assert_example_workspace_clean("ecommerce").await;
}

#[tokio::test]
async fn functions_demo() {
    assert_example_workspace_clean("functions_demo").await;
}

#[tokio::test]
async fn meta_lists() {
    assert_example_workspace_clean("meta_lists").await;
}

#[tokio::test]
async fn meta_hofs() {
    assert_example_workspace_clean("meta_hofs").await;
}

#[tokio::test]
async fn meta_columns() {
    assert_example_workspace_clean("meta_columns").await;
}

#[tokio::test]
async fn meta_workspace() {
    assert_example_workspace_clean("meta_workspace").await;
}

#[tokio::test]
async fn meta_polish() {
    assert_example_workspace_clean("meta_polish").await;
}

#[tokio::test]
async fn meta_config() {
    assert_example_workspace_clean("meta_config").await;
}

#[tokio::test]
async fn per_cohort_union() {
    assert_example_workspace_clean("per_cohort_union").await;
}

#[tokio::test]
async fn staging_from_sources() {
    assert_example_workspace_clean("staging_from_sources").await;
}
