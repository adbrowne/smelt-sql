//! Standing gate: the editor's property-diff surface (code lens titles,
//! `PropertyDowngrade` diagnostics) must agree with the CLI's `DiffReport`
//! for the same working tree (`docs/specs/property_diff.md` §Constraints
//! item 5, "Surface parity";
//! `docs/outcomes/20260905-property-diff/phases/07-plan.md` ruling R3).
//!
//! The two sides are genuinely different code paths, not one function
//! rendered twice:
//! - The CLI side calls `smelt_runtime::property_diff::{work_side,
//!   baseline_side, report}` directly and serializes the resulting
//!   `DiffReport` — the exact value `smelt explain --diff --json` prints.
//! - The LSP side is read OFF THE WIRE: a real `textDocument/codeLens`
//!   response and real `publishDiagnostics` notifications from a real
//!   `Backend` driven over duplex streams (same harness as
//!   `example_workspaces.rs`), after project routing, model-name→path
//!   mapping, per-file aggregation, anchoring, and caching — none of which
//!   the CLI side exercises.
//!
//! Hard-coded non-emptiness assertions come BEFORE the cross-check, so an
//! empty-vs-empty comparison fails at the first assertion rather than the
//! (trivially true) set equality (ruling R3).
//!
//! **Sabotage-run record** (ruling R3, required once per implementation):
//! the LSP-side per-file diagnostic list was manually truncated (dropping
//! the last downgrade) to confirm this gate goes red before being
//! committed green. See `docs/outcomes/20260905-property-diff/phases/
//! 07-summary.md` for the exact mutation and the resulting failure
//! message.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tower_lsp::{LspService, Server};

use smelt_lsp::Backend;

// ---------------------------------------------------------------------------
// Git fixture helpers (re-created from `crates/smelt-cli/tests/
// property_diff_cli.rs` — a test-binary-local module cannot be imported
// across crates).
// ---------------------------------------------------------------------------

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

/// Stage `examples/timeseries` as a fresh git repo committed on `main`,
/// then apply the join-induced downgrade edit to `user_daily_spend.sql`
/// (verified by hand — see `crates/smelt-cli/tests/property_diff_cli.rs`'s
/// `a_join_induced_downgrade_propagates_to_the_named_downstream_model`).
/// Returns the repo root (== project dir).
fn stage_edited_timeseries_repo(tmp: &Path) {
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

    let model_path = tmp.join("models/user_daily_spend.sql");
    let original = std::fs::read_to_string(&model_path).expect("read user_daily_spend.sql");
    let edited = original.replace(
        "SELECT\n    user_id,\n    CAST(transaction_timestamp AS DATE) AS spend_date,\n    SUM(amount) AS total_amount\nFROM smelt.sources.raw.transactions\nGROUP BY 1, 2",
        "SELECT\n    t.user_id,\n    CAST(t.transaction_timestamp AS DATE) AS spend_date,\n    SUM(t.amount) AS total_amount\nFROM smelt.sources.raw.transactions t\nJOIN smelt.sources.raw.users u ON t.user_id = u.user_id\nGROUP BY 1, 2",
    );
    assert_ne!(
        original, edited,
        "the fixture's SELECT text must match what this test replaces"
    );
    std::fs::write(&model_path, edited).expect("write edited user_daily_spend.sql");
}

// ---------------------------------------------------------------------------
// LSP protocol helpers (duplicated from `example_workspaces.rs` per its own
// comment: cheaper than sharing across a private test-module boundary).
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

    /// Drain any notifications sitting on the wire without blocking (used
    /// while polling for `code_lens` to become non-empty, so
    /// `publishDiagnostics` notifications aren't left stuck ahead of the
    /// next `id`-matched response in `send_request`'s read loop).
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

/// Poll `textDocument/codeLens` for `path` until it returns a non-empty
/// array or `timeout_ms` elapses. `refresh_property_diff` runs off the
/// request path (git subprocesses + workspace derivation), so the lens is
/// not necessarily populated the instant `initialized` returns.
async fn poll_code_lens(client: &mut TestClient, uri: &str, timeout_ms: u64) -> Vec<Value> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    loop {
        let result = client
            .send_request(
                "textDocument/codeLens",
                json!({ "textDocument": { "uri": uri } }),
            )
            .await;
        client.drain_pending().await;
        if let Some(lenses) = result.as_array() {
            if !lenses.is_empty() {
                return lenses.clone();
            }
        }
        if std::time::Instant::now() >= deadline {
            return Vec::new();
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn editor_lens_and_diagnostics_agree_with_the_cli_report() {
    let tmp = tempfile::tempdir().expect("tempdir");
    stage_edited_timeseries_repo(tmp.path());

    // --- CLI side: the exact pipeline `smelt explain --diff --json` uses. ---
    let work = smelt_runtime::property_diff::work_side(tmp.path(), &BTreeMap::new())
        .expect("work_side must derive on the edited fixture");
    let base = smelt_runtime::property_diff::baseline_side(tmp.path(), None)
        .expect("baseline_side must resolve the default merge-base");
    let cli_report = smelt_runtime::property_diff::report(&work, &base);

    // Hard-coded non-emptiness BEFORE any cross-check (ruling R3): an
    // empty-vs-empty comparison must fail here, not at a trivial equality
    // further down.
    assert!(
        cli_report.summary.shifted_models >= 1,
        "fixture must shift at least one model: {:?}",
        cli_report.summary
    );
    assert!(
        cli_report.summary.downgrades >= 1,
        "fixture must contain at least one downgrade: {:?}",
        cli_report.summary
    );
    assert!(
        cli_report
            .models
            .iter()
            .any(|m| m.model == "user_daily_spend"),
        "user_daily_spend must be named as shifted: {:?}",
        cli_report
            .models
            .iter()
            .map(|m| &m.model)
            .collect::<Vec<_>>()
    );
    let unshifted_names: Vec<&str> = work
        .loaded
        .sql_files
        .iter()
        .map(|m| m.name.as_str())
        .filter(|name| !cli_report.models.iter().any(|m| m.model == *name))
        .collect();
    assert!(
        !unshifted_names.is_empty(),
        "fixture must leave at least one model unshifted"
    );
    let unshifted_model = unshifted_names[0];

    // The CLI JSON's `stories` for `user_daily_spend` — the same value
    // `smelt explain --diff --json` prints — is the oracle for both the
    // lens title and the `PropertyDowngrade` set (§Constraints item 5).
    let cli_json = serde_json::to_value(&cli_report).expect("DiffReport must serialize");
    let cli_model_json = cli_json["models"]
        .as_array()
        .expect("models must be an array")
        .iter()
        .find(|m| m["model"] == "user_daily_spend")
        .expect("user_daily_spend must be present in the JSON")
        .clone();
    let cli_stories_json = cli_model_json["stories"]
        .as_array()
        .expect("stories must be an array")
        .clone();
    let cli_risk_or_cost_messages: std::collections::BTreeSet<String> = cli_stories_json
        .iter()
        .filter(|s| s["severity"] == "risk" || s["severity"] == "cost")
        .map(|s| {
            format!(
                "{}: {}",
                s["lead"].as_str().unwrap(),
                s["detail"].as_str().unwrap()
            )
        })
        .collect();
    assert!(
        !cli_risk_or_cost_messages.is_empty(),
        "fixture must produce at least one risk/cost story for user_daily_spend: {cli_stories_json:?}"
    );
    let cli_lens_title = smelt_logical::analysis::diff_stories::lens_title(
        cli_report
            .models
            .iter()
            .find(|m| m.model == "user_daily_spend")
            .unwrap(),
        &cli_report.baseline,
    );

    // --- LSP side: read off the wire, over a real Backend. ---
    let mut client = TestClient::open_workspace(tmp.path()).await;
    let model_uri = format!(
        "file://{}",
        tmp.path().join("models/user_daily_spend.sql").display()
    );
    let unshifted_uri = format!(
        "file://{}",
        tmp.path()
            .join(format!("models/{unshifted_model}.sql"))
            .display()
    );

    let lenses = poll_code_lens(&mut client, &model_uri, 30_000).await;
    assert_eq!(
        lenses.len(),
        1,
        "a shifted model must carry exactly one lens: {lenses:?}"
    );
    let lens_title = lenses[0]["command"]["title"].as_str().unwrap().to_string();
    assert_eq!(
        lens_title, cli_lens_title,
        "lens title must match the CLI's lens_title primitive exactly"
    );

    let unshifted_lenses = client
        .send_request(
            "textDocument/codeLens",
            json!({ "textDocument": { "uri": unshifted_uri } }),
        )
        .await;
    let unshifted_lenses = unshifted_lenses.as_array().cloned().unwrap_or_default();
    assert!(
        unshifted_lenses.is_empty(),
        "an unshifted model must carry no lens: {unshifted_lenses:?}"
    );

    // publishDiagnostics: give the server a moment to flush the diagnostics
    // it publishes as part of the refresh that populated the lens above.
    client.drain_pending().await;
    let downgrade_diagnostics: Vec<Value> = client
        .diagnostics_for("models/user_daily_spend.sql")
        .into_iter()
        .filter(|d| d["code"] == "property-downgrade")
        .collect();
    let lsp_messages: std::collections::BTreeSet<String> = downgrade_diagnostics
        .iter()
        .map(|d| d["message"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        lsp_messages, cli_risk_or_cost_messages,
        "the PropertyDowngrade message set must equal the CLI JSON's risk/cost stories \
         for user_daily_spend: lsp={downgrade_diagnostics:?}"
    );
    assert!(
        downgrade_diagnostics
            .iter()
            .all(|d| d["severity"] == 2 /* Warning */),
        "PropertyDowngrade diagnostics must be Warning severity: {downgrade_diagnostics:?}"
    );

    let unshifted_downgrade_diagnostics: Vec<Value> = client
        .diagnostics_for(&format!("models/{unshifted_model}.sql"))
        .into_iter()
        .filter(|d| d["code"] == "property-downgrade")
        .collect();
    assert!(
        unshifted_downgrade_diagnostics.is_empty(),
        "an unshifted model must carry no PropertyDowngrade diagnostic: {unshifted_downgrade_diagnostics:?}"
    );
}
