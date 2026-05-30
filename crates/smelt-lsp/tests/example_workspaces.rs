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
            // 60s per-RPC timeout: cargo runs tests in parallel within the
            // binary; multiple multi-project tests open all of `examples/`
            // simultaneously and compete for CPU on single-vCPU CI runners.
            // Initialize takes ~4s locally and ~30s on a contended runner.
            // 60s is generous enough to absorb that contention while still
            // bounding genuine hangs.
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

    async fn references(
        &mut self,
        file_uri: &str,
        line: u32,
        character: u32,
    ) -> Vec<lsp_types::Location> {
        let result = self
            .send_request(
                "textDocument/references",
                json!({
                    "textDocument": { "uri": file_uri },
                    "position": { "line": line, "character": character },
                    "context": { "includeDeclaration": true }
                }),
            )
            .await;
        if result.is_null() {
            return Vec::new();
        }
        serde_json::from_value(result).unwrap_or_default()
    }

    async fn hover(&mut self, file_uri: &str, line: u32, character: u32) -> Option<String> {
        let result = self
            .send_request(
                "textDocument/hover",
                json!({
                    "textDocument": { "uri": file_uri },
                    "position": { "line": line, "character": character }
                }),
            )
            .await;
        if result.is_null() {
            return None;
        }
        let contents = result.get("contents")?;
        contents
            .get("value")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
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

// ---------------------------------------------------------------------------
// Multi-project workspace case — the project isolation rule.
// ---------------------------------------------------------------------------

/// Find-references on a `smelt.functions.<name>(...)` call site returns at
/// least the call site itself. Uses `parse_event_payload`, called from
/// `models/silver/events_parsed.sql`. Reproduces the VSCode case the user hit:
/// open `examples/web_analytics/` as the workspace folder and "Find All
/// References" on the `sessionize` token inside the
/// `smelt.functions.sessionize(...)` call in `models/silver/sessions.sql`.
#[tokio::test]
async fn find_references_on_function_call_in_web_analytics() {
    let workspace = examples_root().join("web_analytics");
    let target_file = workspace
        .join("models")
        .join("silver")
        .join("events_parsed.sql");
    assert!(
        target_file.exists(),
        "fixture not found: {}",
        target_file.display()
    );

    let mut client = TestClient::open_workspace(&workspace).await;
    client
        .open_file(&target_file)
        .await
        .expect("open events_parsed.sql");

    // Drain initial diagnostics so subsequent requests don't race with them.
    let _ = client.collect_diagnostics(1000).await;

    // models/silver/events_parsed.sql line 27 (1-indexed):
    //     smelt.functions.parse_event_payload(payload).*
    // Cursor on "parse_event_payload" word — char 25 is inside the token.
    let file_uri = format!("file://{}", target_file.display());
    let locations = client.references(&file_uri, 26, 25).await;
    client.shutdown().await;

    assert!(
        !locations.is_empty(),
        "expected at least one reference for smelt.functions.parse_event_payload, got 0"
    );
    let on_target = locations
        .iter()
        .any(|loc| loc.uri.as_str().contains("events_parsed.sql"));
    assert!(
        on_target,
        "expected the call site in events_parsed.sql among references; got: {:?}",
        locations
    );
}

/// Find-references on the `smelt.define <name>` declaration name token
/// returns the call sites in the same project. This mirrors the VSCode
/// case where the user puts the cursor on `parse_event_payload` inside
/// `examples/web_analytics/functions/parse_event_payload.sql`'s
/// `smelt.define parse_event_payload(...)` line and asks "find all callers".
#[tokio::test]
async fn find_references_from_function_definition_in_web_analytics() {
    let workspace = examples_root().join("web_analytics");
    let target_file = workspace.join("functions").join("parse_event_payload.sql");
    assert!(target_file.exists());

    let mut client = TestClient::open_workspace(&workspace).await;
    client
        .open_file(&target_file)
        .await
        .expect("open parse_event_payload.sql");
    let _ = client.collect_diagnostics(1000).await;

    // Line 5 (1-indexed) = line 4 (0-indexed) is `smelt.define parse_event_payload(`.
    // The `parse_event_payload` token starts at character 13.
    let file_uri = format!("file://{}", target_file.display());
    let locations = client.references(&file_uri, 4, 16).await;
    client.shutdown().await;

    assert!(
        !locations.is_empty(),
        "expected ≥1 reference for the parse_event_payload define-name token, got 0"
    );
    let has_call_site = locations
        .iter()
        .any(|loc| loc.uri.as_str().contains("events_parsed.sql"));
    assert!(
        has_call_site,
        "expected the events_parsed.sql call site among references; got: {:?}",
        locations
    );
}

/// Standing CI gate for the project isolation rule
/// (`docs/specs/architecture.md` → "Project isolation rule").
///
/// VSCode opens a monorepo folder once; the LSP discovers every smelt
/// project under it via `find_smelt_projects`. Without project-scoped
/// resolution, a function-name collision across two projects (e.g.
/// `examples/web_analytics/functions/sessionize.sql` vs
/// `examples/functions_demo/functions/sessionize.sql`, both named
/// `sessionize` but with different parameter lists) lets one project's
/// signature win for both call sites — producing a spurious
/// `Missing required argument` on the project whose call doesn't match
/// the winning signature.
///
/// This test opens the entire `examples/` directory as ONE workspace
/// folder (matching the VSCode monorepo case) and asserts the
/// `Missing required argument` does not appear on
/// `web_analytics/models/silver/sessions.sql`.
#[tokio::test]
async fn project_isolation_in_multi_project_workspace() {
    let examples = examples_root();
    let target_file = examples
        .join("web_analytics")
        .join("models")
        .join("silver")
        .join("sessions.sql");
    assert!(
        target_file.exists(),
        "fixture not found: {}",
        target_file.display()
    );

    // Open `examples/` as the workspace folder. `find_smelt_projects`
    // will discover every example workspace (web_analytics,
    // functions_demo, timeseries, ...) inside it. Each becomes a
    // ProjectInput in the workspace; the isolation rule says
    // resolve_function in one project must not see another's signatures.
    let mut client = TestClient::open_workspace(&examples).await;

    // Open just the target file. We don't need to open every model in
    // every project — opening sessions.sql triggers diagnostics for it.
    client
        .open_file(&target_file)
        .await
        .expect("open sessions.sql");

    let diags = client.collect_diagnostics(3000).await;
    client.shutdown().await;

    // Find the LATEST diagnostic batch for sessions.sql (subsequent
    // publishes supersede earlier ones).
    let target_uri_substr = "web_analytics/models/silver/sessions.sql";
    let latest: Vec<&lsp_types::Diagnostic> = diags
        .iter()
        .rfind(|(uri, _)| uri.contains(target_uri_substr))
        .map(|(_, ds)| ds.iter().collect())
        .unwrap_or_default();

    let missing_arg: Vec<&lsp_types::Diagnostic> = latest
        .iter()
        .copied()
        .filter(|d| d.message.contains("Missing required argument"))
        .collect();

    assert!(
        missing_arg.is_empty(),
        "project isolation violated: `Missing required argument` fired on \
         sessions.sql when examples/ was opened as a multi-project workspace. \
         Diagnostics: {:?}",
        missing_arg,
    );
}

/// Project-isolation CI gate for **model-name resolution** (the
/// `resolve_ref` axis, distinct from the `resolve_function` axis the test
/// above covers).
///
/// Repro: `examples/web_analytics/models/sources/raw/events.yml` and
/// `examples/timeseries/models/events.sql` both share the leaf name
/// `events` but live in different projects with different schemas. Opening
/// `examples/` as one workspace folder used to make `resolve_ref("events")`
/// — called from `process_table_ref_pure` while building the type context
/// for a file that does `FROM smelt.sources.raw.events` — return the
/// timeseries model's columns to the web-analytics file, poisoning column
/// type inference for any column whose name happens to exist in both
/// schemas (here: `event_id`, `user_id`). That surfaced as a
/// `Could not infer type` warning on those specific columns of
/// `web_analytics/models/bronze/raw_events.sql`.
///
/// The fix threads `ProjectInput` through `resolve_ref` (mirroring the
/// `resolve_function` fix). This test asserts no cross-project column-type
/// leak on raw_events.sql.
#[tokio::test]
async fn no_cross_project_column_type_leak_through_resolve_ref() {
    let examples = examples_root();
    let target_file = examples
        .join("web_analytics")
        .join("models")
        .join("bronze")
        .join("raw_events.sql");
    assert!(
        target_file.exists(),
        "fixture not found: {}",
        target_file.display()
    );

    let mut client = TestClient::open_workspace(&examples).await;
    client
        .open_file(&target_file)
        .await
        .expect("open raw_events.sql");

    let diags = client.collect_diagnostics(3000).await;
    client.shutdown().await;

    let target_uri_substr = "web_analytics/models/bronze/raw_events.sql";
    let latest: Vec<&lsp_types::Diagnostic> = diags
        .iter()
        .rfind(|(uri, _)| uri.contains(target_uri_substr))
        .map(|(_, ds)| ds.iter().collect())
        .unwrap_or_default();

    let leaks: Vec<&lsp_types::Diagnostic> = latest
        .iter()
        .copied()
        .filter(|d| {
            d.message.contains("Could not infer type")
                || d.message.contains("Undeclared column")
                || d.message.contains("Undefined source")
        })
        .collect();

    assert!(
        leaks.is_empty(),
        "project isolation violated: type-inference / column diagnostics \
         fired on raw_events.sql when examples/ was opened as a \
         multi-project workspace. Every column listed in raw_events.sql \
         is declared in web_analytics's events.yml; any diagnostic here \
         is a cross-project leak. Diagnostics: {:?}",
        leaks,
    );
}

/// Hover on a `smelt.sources.<schema>.<table>` path ref must render the
/// per-entity source YAML's description + columns, not fall back to the
/// "Undefined source" branch.
///
/// Regression: the hover handler only consulted the legacy aggregate
/// `sources.yml` resolver; projects that use per-entity `models/sources/.../*.yml`
/// files (the canonical layout since the per-entity migration) would always
/// hit the "Undefined source" fallback. Reproduced by opening
/// `web_analytics/models/bronze/raw_events.sql` and hovering on the
/// `events` token of `FROM smelt.sources.raw.events`.
#[tokio::test]
async fn hover_on_per_entity_source_renders_columns() {
    let workspace = examples_root().join("web_analytics");
    let target_file = workspace
        .join("models")
        .join("bronze")
        .join("raw_events.sql");
    assert!(
        target_file.exists(),
        "fixture not found: {}",
        target_file.display()
    );

    let mut client = TestClient::open_workspace(&workspace).await;
    client
        .open_file(&target_file)
        .await
        .expect("open raw_events.sql");
    let _ = client.collect_diagnostics(1000).await;

    // raw_events.sql line 10 (1-indexed) = line 9 (0-indexed):
    //   `FROM smelt.sources.raw.events`
    // Character 25 lands inside the `events` token.
    let file_uri = format!("file://{}", target_file.display());
    let hover = client.hover(&file_uri, 9, 25).await;
    client.shutdown().await;

    let content = hover.expect("hover on smelt.sources.raw.events must return content");
    assert!(
        !content.contains("Undefined source"),
        "hover on smelt.sources.raw.events returned `Undefined source` even \
         though web_analytics declares the source at \
         models/sources/raw/events.yml (per-entity layout). Hover content:\n{}",
        content
    );
    // Should render columns from the per-entity YAML.
    assert!(
        content.contains("event_id") && content.contains("device_id"),
        "hover should render columns from per-entity events.yml. Got:\n{}",
        content
    );
}
