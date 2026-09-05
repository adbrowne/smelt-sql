//! S2 (Phase 7 review fix round 1): Δ2's open-buffer promise
//! (`docs/specs/property_diff.md` §Surface "Editor" — "Open buffers
//! override on-disk contents for model files on the working-tree side")
//! had zero LSP-level coverage. This drives a real `Backend` over duplex
//! streams (same harness shape as `property_diff_parity.rs`), opens a
//! model document, edits it in the buffer WITHOUT saving, forces a
//! refresh via an unrelated single-event `.git` notification, and asserts
//! the lens/diagnostic set reflects the unsaved edit while the on-disk
//! file is untouched.

use std::path::Path;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tower_lsp::{LspService, Server};

use smelt_lsp::Backend;

fn git(dir: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git must be available to run these tests");
    assert!(
        output.status.success(),
        "git {:?} failed in {:?}: {}",
        args,
        dir,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_commit(dir: &Path, message: &str) {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "user.email=t@example.invalid",
            "-c",
            "user.name=t",
            "commit",
            "-m",
            message,
        ])
        .current_dir(dir)
        .output()
        .expect("git must be available to run these tests");
    assert!(
        output.status.success(),
        "git commit failed in {:?}: {}",
        dir,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in std::fs::read_dir(src).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name == "target" || name == ".smelt" || name == ".git" {
            continue;
        }
        let dest = dst.join(&name);
        if path.is_dir() {
            copy_dir(&path, &dest);
        } else {
            std::fs::copy(&path, &dest).unwrap();
        }
    }
}

/// Stage a plain, unedited `examples/timeseries` repo committed on `main`
/// — the disk content of `user_daily_spend.sql` matches the baseline
/// exactly, so any lens/diagnostic seen on it must come from the buffer
/// overlay, not the file system.
fn stage_plain_timeseries_repo(tmp: &Path) {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/timeseries");
    copy_dir(&repo_root, tmp);
    git(tmp, &["init", "-q", "-b", "main"]);
    git(tmp, &["add", "-A"]);
    git_commit(tmp, "initial import of examples/timeseries");
}

// ---------------------------------------------------------------------------
// LSP protocol helpers (duplicated per this suite's own convention).
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

    async fn drain_pending(&mut self) {
        while let Some(msg) = read_message_timeout(&mut self.client_rx, 50).await {
            self.notifications.push(msg);
        }
    }

    fn diagnostics_for(&self, uri_suffix: &str) -> Vec<Value> {
        self.notifications
            .iter()
            .filter(|n| {
                n.get("method").and_then(|m| m.as_str()) == Some("textDocument/publishDiagnostics")
            })
            .filter(|n| {
                n["params"]["uri"]
                    .as_str()
                    .is_some_and(|u| u.ends_with(uri_suffix))
            })
            .flat_map(|n| {
                n["params"]["diagnostics"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .collect()
    }
}

async fn code_lens_for(client: &mut TestClient, uri: &str) -> Vec<Value> {
    let result = client
        .send_request(
            "textDocument/codeLens",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await;
    result.as_array().cloned().unwrap_or_default()
}

/// Trigger a property-diff refresh without saving or touching the model's
/// own file: a single (non-burst — see `property_diff_coalescing.rs`'s doc
/// comment on why a multi-event notification is avoided over the wire)
/// `.git/HEAD`-path `didChangeWatchedFiles` event, which the spec's Δ2
/// refresh triggers cover regardless of whether HEAD actually moved
/// (`refresh_property_diff` always re-resolves and compares).
async fn force_refresh(client: &mut TestClient, project_root: &Path) {
    let uri = format!("file://{}", project_root.join(".git/HEAD").display());
    client
        .send_notification(
            "workspace/didChangeWatchedFiles",
            json!({ "changes": [{ "uri": uri, "type": 2 }] }),
        )
        .await;
}

async fn poll_until<F>(client: &mut TestClient, uri: &str, timeout_ms: u64, mut ok: F) -> Vec<Value>
where
    F: FnMut(&[Value]) -> bool,
{
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    loop {
        let lenses = code_lens_for(client, uri).await;
        client.drain_pending().await;
        if ok(&lenses) {
            return lenses;
        }
        if std::time::Instant::now() >= deadline {
            return lenses;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn an_unsaved_buffer_edit_changes_the_lens_and_diagnostics_without_touching_disk() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_plain_timeseries_repo(tmp.path());

    let model_path = tmp.path().join("models/user_daily_spend.sql");
    let original = std::fs::read_to_string(&model_path).expect("read user_daily_spend.sql");
    // The same join-induced downgrade edit `property_diff_parity.rs` uses,
    // applied to the BUFFER only — `model_path` on disk is never written.
    let edited = original.replace(
        "SELECT\n    user_id,\n    CAST(transaction_timestamp AS DATE) AS spend_date,\n    SUM(amount) AS total_amount\nFROM smelt.sources.raw.transactions\nGROUP BY 1, 2",
        "SELECT\n    t.user_id,\n    CAST(t.transaction_timestamp AS DATE) AS spend_date,\n    SUM(t.amount) AS total_amount\nFROM smelt.sources.raw.transactions t\nJOIN smelt.sources.raw.users u ON t.user_id = u.user_id\nGROUP BY 1, 2",
    );
    assert_ne!(
        original, edited,
        "the fixture's SELECT text must match what this test replaces"
    );

    let mut client = TestClient::open_workspace(tmp.path()).await;
    let model_uri = format!("file://{}", model_path.display());

    // Before any edit: the on-disk content matches the baseline exactly,
    // so the model must be unshifted — no lens. Give the initial
    // workspace-load refresh (triggered by `initialized`) a moment to
    // settle first.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    client.drain_pending().await;
    assert!(
        code_lens_for(&mut client, &model_uri).await.is_empty(),
        "an untouched model must carry no lens before the buffer edit"
    );

    // Open the document (original content) then edit it in the buffer —
    // full-document sync, matching the server's advertised
    // `TextDocumentSyncKind::FULL` — WITHOUT ever sending `didSave`.
    client
        .send_notification(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": model_uri,
                    "languageId": "sql",
                    "version": 1,
                    "text": original,
                }
            }),
        )
        .await;
    client
        .send_notification(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": model_uri, "version": 2 },
                "contentChanges": [{ "text": edited }]
            }),
        )
        .await;

    force_refresh(&mut client, tmp.path()).await;

    let lenses_after_edit = poll_until(&mut client, &model_uri, 30_000, |l| !l.is_empty()).await;
    assert_eq!(
        lenses_after_edit.len(),
        1,
        "the unsaved buffer edit must produce a lens even though the disk file is untouched: {lenses_after_edit:?}"
    );

    client.drain_pending().await;
    let downgrade_diagnostics: Vec<Value> = client
        .diagnostics_for("models/user_daily_spend.sql")
        .into_iter()
        .filter(|d| d["code"] == "property-downgrade")
        .collect();
    assert!(
        !downgrade_diagnostics.is_empty(),
        "the unsaved edit's downgrade must be diagnosed"
    );

    // The disk file itself was never written.
    let on_disk_after = std::fs::read_to_string(&model_path).expect("read again");
    assert_eq!(
        on_disk_after, original,
        "apply_open_buffers must never write the overlay back to disk"
    );
}
