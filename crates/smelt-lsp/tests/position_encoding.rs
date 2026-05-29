//! Standing CI gate: `positionEncodingKind` negotiation and non-ASCII position
//! correctness.
//!
//! Covers:
//! - Encoding negotiation via `InitializeParams.capabilities.general.positionEncodings`.
//! - Non-ASCII fixture: `examples/non_ascii_columns/` has Greek-letter column
//!   aliases; the test verifies zero diagnostics under both encodings.
//! - ASCII baseline: `examples/per_cohort_union/` produces byte-identical ranges
//!   under both UTF-8 and UTF-16 (columns are ASCII-only).
//! - Diagnostic character values differ for non-ASCII under UTF-8 vs UTF-16.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use smelt_lsp::Backend;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tower_lsp::{LspService, Server};

// ---------------------------------------------------------------------------
// Protocol helpers (self-contained copy, same pattern as example_workspaces.rs)
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
    /// Boot the LSP with a given `position_encodings` client capability list.
    /// Pass `None` to simulate a client that advertises no preference.
    async fn open_workspace_with_encoding(
        workspace_dir: &Path,
        position_encodings: Option<&[&str]>,
    ) -> (Self, Value) {
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

        let general_caps = if let Some(encodings) = position_encodings {
            json!({ "positionEncodings": encodings })
        } else {
            json!({})
        };

        let init_result = client
            .send_request(
                "initialize",
                json!({
                    "processId": null,
                    "capabilities": {
                        "workspace": { "workspaceFolders": true },
                        "general": general_caps
                    },
                    "workspaceFolders": [{ "uri": workspace_uri, "name": "test" }]
                }),
            )
            .await;

        assert!(
            init_result.get("capabilities").is_some(),
            "initialize should return capabilities"
        );
        client.send_notification("initialized", json!({})).await;

        (client, init_result)
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

fn examples_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root")
        .join("examples")
}

// ---------------------------------------------------------------------------
// Test 1: Negotiation — client advertises UTF-8 → server picks UTF-8
// ---------------------------------------------------------------------------

#[tokio::test]
async fn negotiation_utf8_client_picks_utf8() {
    let workspace = examples_root().join("non_ascii_columns");
    let (mut client, init_result) =
        TestClient::open_workspace_with_encoding(&workspace, Some(&["utf-8"])).await;

    let negotiated = init_result["capabilities"]["positionEncoding"]
        .as_str()
        .unwrap_or("");
    assert_eq!(
        negotiated, "utf-8",
        "server should negotiate utf-8 when client requests it; got {:?}",
        negotiated
    );

    client.shutdown().await;
}

// ---------------------------------------------------------------------------
// Test 2: Negotiation — client advertises UTF-16 → server picks UTF-16
// ---------------------------------------------------------------------------

#[tokio::test]
async fn negotiation_utf16_client_picks_utf16() {
    let workspace = examples_root().join("non_ascii_columns");
    let (mut client, init_result) =
        TestClient::open_workspace_with_encoding(&workspace, Some(&["utf-16"])).await;

    let negotiated = init_result["capabilities"]["positionEncoding"]
        .as_str()
        .unwrap_or("");
    assert_eq!(
        negotiated, "utf-16",
        "server should negotiate utf-16 when client requests it; got {:?}",
        negotiated
    );

    client.shutdown().await;
}

// ---------------------------------------------------------------------------
// Test 3: Negotiation — client advertises no preference → server uses UTF-16
// ---------------------------------------------------------------------------

#[tokio::test]
async fn negotiation_no_client_preference_defaults_to_utf16() {
    let workspace = examples_root().join("non_ascii_columns");
    // Pass None: client sends no `general.positionEncodings` field.
    let (mut client, init_result) =
        TestClient::open_workspace_with_encoding(&workspace, None).await;

    let negotiated = init_result["capabilities"]["positionEncoding"]
        .as_str()
        .unwrap_or(""); // empty string so absence is caught — see Finding 3
    assert_eq!(
        negotiated, "utf-16",
        "server should default to utf-16 when client advertises no preference; got {:?}",
        negotiated
    );

    client.shutdown().await;
}

// ---------------------------------------------------------------------------
// Test 4: Negotiation — client advertises unsupported encoding → server falls
// back to UTF-16
// ---------------------------------------------------------------------------

#[tokio::test]
async fn negotiation_unsupported_encoding_falls_back_to_utf16() {
    let workspace = examples_root().join("non_ascii_columns");
    // "utf-32" is not supported by this server.
    let (mut client, init_result) =
        TestClient::open_workspace_with_encoding(&workspace, Some(&["utf-32"])).await;

    let negotiated = init_result["capabilities"]["positionEncoding"]
        .as_str()
        .unwrap_or(""); // empty string so absence is caught — see Finding 3
    assert_eq!(
        negotiated, "utf-16",
        "server should fall back to utf-16 for unsupported encoding; got {:?}",
        negotiated
    );

    client.shutdown().await;
}

// ---------------------------------------------------------------------------
// Test 5: Non-ASCII fixture — zero diagnostics under UTF-8 negotiation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_ascii_fixture_clean_under_utf8() {
    let workspace = examples_root().join("non_ascii_columns");
    let greek_sql = workspace.join("models/greek.sql");

    let (mut client, _) =
        TestClient::open_workspace_with_encoding(&workspace, Some(&["utf-8"])).await;
    client.open_file(&greek_sql).await.unwrap();

    let all_diags = client.collect_diagnostics(3000).await;
    let errors: Vec<_> = all_diags
        .iter()
        .flat_map(|(uri, diags)| diags.iter().map(move |d| (uri, d)))
        .filter(|(_, d)| {
            d.severity == Some(lsp_types::DiagnosticSeverity::ERROR)
                || d.severity == Some(lsp_types::DiagnosticSeverity::WARNING)
        })
        .collect();

    assert!(
        errors.is_empty(),
        "non_ascii_columns fixture should have zero diagnostics under utf-8; got: {:?}",
        errors
    );

    client.shutdown().await;
}

// ---------------------------------------------------------------------------
// Test 6: Non-ASCII fixture — zero diagnostics under UTF-16 negotiation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_ascii_fixture_clean_under_utf16() {
    let workspace = examples_root().join("non_ascii_columns");
    let greek_sql = workspace.join("models/greek.sql");

    let (mut client, _) =
        TestClient::open_workspace_with_encoding(&workspace, Some(&["utf-16"])).await;
    client.open_file(&greek_sql).await.unwrap();

    let all_diags = client.collect_diagnostics(3000).await;
    let errors: Vec<_> = all_diags
        .iter()
        .flat_map(|(uri, diags)| diags.iter().map(move |d| (uri, d)))
        .filter(|(_, d)| {
            d.severity == Some(lsp_types::DiagnosticSeverity::ERROR)
                || d.severity == Some(lsp_types::DiagnosticSeverity::WARNING)
        })
        .collect();

    assert!(
        errors.is_empty(),
        "non_ascii_columns fixture should have zero diagnostics under utf-16; got: {:?}",
        errors
    );

    client.shutdown().await;
}

// ---------------------------------------------------------------------------
// Test 7: ASCII baseline — a broken ASCII-only fixture produces byte-identical
// ranges under both encodings (no multi-byte characters means byte offset ==
// UTF-16 code unit offset).
// ---------------------------------------------------------------------------

/// Collect all diagnostic ranges from the given workspace under the given encoding.
async fn collect_ranges_for_workspace(workspace: &Path, encoding: &str) -> Vec<lsp_types::Range> {
    let files: Vec<PathBuf> = {
        let models_dir = workspace.join("models");
        walkdir::WalkDir::new(&models_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "sql").unwrap_or(false))
            .map(|e| e.path().to_path_buf())
            .collect()
    };

    let (mut client, _) =
        TestClient::open_workspace_with_encoding(workspace, Some(&[encoding])).await;
    for f in &files {
        let _ = client.open_file(f).await;
    }

    let all_diags = client.collect_diagnostics(3000).await;
    client.shutdown().await;

    all_diags
        .into_iter()
        .flat_map(|(_, diags)| diags.into_iter().map(|d| d.range))
        .collect()
}

#[tokio::test]
async fn ascii_baseline_byte_identical_ranges_under_both_encodings() {
    // Use `per_cohort_union_broken_emission_body_undeclared_column`: an ASCII-only
    // broken workspace that produces at least one diagnostic. Comparing the ranges
    // under UTF-8 and UTF-16 confirms that ASCII positions are encoding-invariant.
    // (Previously this used the clean `per_cohort_union` workspace, which produced
    // an empty-vs-empty comparison — a vacuous assertion.)
    let workspace = examples_root().join("per_cohort_union_broken_emission_body_undeclared_column");
    let ranges_utf8 = collect_ranges_for_workspace(&workspace, "utf-8").await;
    let ranges_utf16 = collect_ranges_for_workspace(&workspace, "utf-16").await;

    // Confirm the workspace is actually broken (at least one diagnostic).
    assert!(
        !ranges_utf8.is_empty(),
        "per_cohort_union_broken_emission_body_undeclared_column should produce at least \
         one diagnostic; got none (test may be vacuous if fixture changed)"
    );

    // All column characters in the fixture are ASCII, so byte offset == UTF-16 offset.
    assert_eq!(
        ranges_utf8, ranges_utf16,
        "ASCII-only broken workspace should produce byte-identical ranges under both encodings"
    );
}

// ---------------------------------------------------------------------------
// Test 8: Non-ASCII synthetic model — UTF-8 character value differs from UTF-16
// for a diagnostic on a column after a 2-byte Greek letter.
//
// SQL: `SELECT 1 AS α, nonexistent FROM smelt.ref()`
// The column `nonexistent` starts at a different byte/UTF-16 offset than
// codepoint offset when a Greek letter precedes it on the same line.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_ascii_utf8_character_differs_from_utf16() {
    use line_index::{LineIndex, WideEncoding};
    use smelt_lsp::diagnostics_boundary::BoundaryConverter;

    // A SQL line where a Greek letter (2 UTF-8 bytes, 1 UTF-16 code unit)
    // precedes the token we care about.
    // Line: `SELECT 1 AS α, 2 AS β`
    // α starts at byte 12, occupies bytes 12-13.
    // β starts at byte 20, occupies bytes 20-21.
    //
    // For a range starting at byte 12 (start of α):
    //   UTF-8 (byte) column = 12
    //   UTF-16 column = 12 (single UTF-16 code unit for α)
    // For a range starting at byte 20 (start of β):
    //   UTF-8 (byte) column = 20
    //   UTF-16 column = 19 (α is 2 bytes but 1 UTF-16 unit → -1 offset)

    let text = "SELECT 1 AS α, 2 AS β\n";
    // Byte layout:
    //   0123456789012  = "SELECT 1 AS "  (12 bytes)
    //   byte 12-13     = α (0xCE 0xB1)
    //   byte 14        = ','
    //   byte 15        = ' '
    //   byte 16        = '2'
    //   byte 17        = ' '
    //   byte 18-19     = "AS"
    //   byte 20        = ' '
    //   byte 21-22     = β (0xCE 0xB2)

    // Verify byte positions using LineIndex directly.
    let li = LineIndex::new(text);

    // Byte offset for the start of β (byte 21):
    let beta_start = text.find('β').expect("β in text") as u32;
    let lc = li.line_col(line_index::TextSize::from(beta_start));
    assert_eq!(lc.line, 0, "β should be on line 0");
    // UTF-8 column (byte offset on the line)
    let utf8_col = lc.col;

    // UTF-16 column (α is 1 UTF-16 code unit but 2 UTF-8 bytes → col is 1 less)
    let wide = li.to_wide(WideEncoding::Utf16, lc).expect("to_wide for β");
    let utf16_col = wide.col;

    assert_ne!(
        utf8_col, utf16_col,
        "UTF-8 byte column ({utf8_col}) should differ from UTF-16 column ({utf16_col}) \
         for a position after a 2-byte Greek letter"
    );
    assert!(
        utf8_col > utf16_col,
        "UTF-8 byte column ({utf8_col}) should be > UTF-16 column ({utf16_col}) \
         (α is 2 bytes but 1 UTF-16 unit)"
    );

    // Now verify BoundaryConverter honours the encoding.
    // Construct a fake Diagnostic range spanning β (just the start for simplicity).
    let fake_range = rowan::TextRange::new(
        rowan::TextSize::from(beta_start),
        rowan::TextSize::from(beta_start + 2), // β is 2 bytes
    );

    let converter_utf8 = BoundaryConverter::new_utf8(text);
    let converter_utf16 = BoundaryConverter::new_utf16(text);

    let range_utf8 = converter_utf8.text_range_to_lsp(fake_range);
    let range_utf16 = converter_utf16.text_range_to_lsp(fake_range);

    assert_eq!(
        range_utf8.start.character, utf8_col,
        "UTF-8 BoundaryConverter should return byte column for β"
    );
    assert_eq!(
        range_utf16.start.character, utf16_col,
        "UTF-16 BoundaryConverter should return UTF-16 column for β"
    );
    assert_ne!(
        range_utf8.start.character, range_utf16.start.character,
        "UTF-8 and UTF-16 BoundaryConverters should produce different character values for non-ASCII text"
    );
}

// ---------------------------------------------------------------------------
// Test 9: E2E non-ASCII diagnostic anchoring — `non_ascii_broken` fixture
//
// The generator body `SELECT 1 AS α, nonexistent_column FROM smelt.upstream`
// has a 2-byte Greek letter (α, U+03B1) before `nonexistent_column`.
//
// Byte layout on the diagnostic line (line 3, 0-indexed):
//   `[ModelDef { name: 'alpha_broken', body: SELECT 1 AS α, nonexistent_column ... }]`
//   The ASCII prefix before α occupies 52 bytes.
//   α occupies bytes 52-53 (2 UTF-8 bytes, 1 UTF-16 code unit).
//   `, ` occupies bytes 54-55 (2 bytes).
//   `nonexistent_column` starts at byte 56 within line 3.
//
//   UTF-8 character = 56  (byte column)
//   UTF-16 character = 55 (α is 2 bytes but 1 UTF-16 unit → -1)
//
// The test initialises the LSP twice (once per encoding), collects
// publishDiagnostics, and asserts:
//   1. Under UTF-8: the UndeclaredColumn diagnostic's start character == 56.
//   2. Under UTF-16: the UndeclaredColumn diagnostic's start character == 55.
//   3. The two character values differ (proving the E2E path honours the encoding).
// ---------------------------------------------------------------------------

/// Expected byte-column (UTF-8) of `nonexistent_column` on line 3 of broken.gen.sql.
///
/// Line 3 (0-indexed): `[ModelDef { name: 'alpha_broken', body: SELECT 1 AS α, nonexistent_column FROM smelt.upstream }]`
///
/// Byte breakdown:
///   `[ModelDef { name: 'alpha_broken', body: SELECT 1 AS ` = 52 ASCII bytes
///   `α` = 2 UTF-8 bytes (U+03B1 → 0xCE 0xB1)
///   `, ` = 2 bytes
///   `nonexistent_column` starts at byte 52 + 2 + 2 = 56
const NON_ASCII_BROKEN_UTF8_CHARACTER: u32 = 56;

/// Expected UTF-16 column of `nonexistent_column` on line 3 of broken.gen.sql.
///
/// α contributes 1 UTF-16 code unit but 2 UTF-8 bytes, so the UTF-16 column
/// is one less than the UTF-8 byte column: 56 - 1 = 55.
const NON_ASCII_BROKEN_UTF16_CHARACTER: u32 = 55;

/// Collect the `UndeclaredColumn` diagnostic ranges from `non_ascii_broken` under
/// the given encoding by driving a real LSP backend over the wire.
async fn collect_non_ascii_broken_undeclared_ranges(encoding: &str) -> Vec<lsp_types::Range> {
    let workspace = examples_root().join("non_ascii_broken");
    let broken_gen = workspace.join("models").join("broken.gen.sql");
    let upstream = workspace.join("models").join("upstream.sql");

    let (mut client, _) =
        TestClient::open_workspace_with_encoding(&workspace, Some(&[encoding])).await;

    // Open upstream first so the schema is available when broken.gen.sql is processed.
    let _ = client.open_file(&upstream).await;
    let _ = client.open_file(&broken_gen).await;

    let all_diags = client.collect_diagnostics(5000).await;
    client.shutdown().await;

    // Collect only UndeclaredColumn ranges (code == "undeclared-column").
    all_diags
        .into_iter()
        .flat_map(|(_, diags)| diags.into_iter())
        .filter(|d| {
            d.code.as_ref().is_some_and(
                |c| matches!(c, lsp_types::NumberOrString::String(s) if s == "undeclared-column"),
            )
        })
        .map(|d| d.range)
        .collect()
}

#[tokio::test]
async fn non_ascii_broken_e2e_utf8_character_correct() {
    let ranges = collect_non_ascii_broken_undeclared_ranges("utf-8").await;
    assert!(
        !ranges.is_empty(),
        "expected at least one UndeclaredColumn diagnostic from non_ascii_broken under utf-8; \
         got none — check that the fixture produces a diagnostic"
    );
    // The diagnostic must land on line 3 (0-indexed).
    let range = &ranges[0];
    assert_eq!(
        range.start.line, 3,
        "UndeclaredColumn should be on line 3 (0-indexed); got line {}",
        range.start.line
    );
    assert_eq!(
        range.start.character, NON_ASCII_BROKEN_UTF8_CHARACTER,
        "Under UTF-8, start.character should be byte column {} for `nonexistent_column` \
         (after 2-byte α); got {}",
        NON_ASCII_BROKEN_UTF8_CHARACTER, range.start.character
    );
}

#[tokio::test]
async fn non_ascii_broken_e2e_utf16_character_correct() {
    let ranges = collect_non_ascii_broken_undeclared_ranges("utf-16").await;
    assert!(
        !ranges.is_empty(),
        "expected at least one UndeclaredColumn diagnostic from non_ascii_broken under utf-16; \
         got none — check that the fixture produces a diagnostic"
    );
    let range = &ranges[0];
    assert_eq!(
        range.start.line, 3,
        "UndeclaredColumn should be on line 3 (0-indexed); got line {}",
        range.start.line
    );
    assert_eq!(
        range.start.character, NON_ASCII_BROKEN_UTF16_CHARACTER,
        "Under UTF-16, start.character should be UTF-16 column {} for `nonexistent_column` \
         (α is 2 UTF-8 bytes but 1 UTF-16 unit); got {}",
        NON_ASCII_BROKEN_UTF16_CHARACTER, range.start.character
    );
}

#[tokio::test]
async fn non_ascii_broken_e2e_utf8_and_utf16_characters_differ() {
    let ranges_utf8 = collect_non_ascii_broken_undeclared_ranges("utf-8").await;
    let ranges_utf16 = collect_non_ascii_broken_undeclared_ranges("utf-16").await;

    assert!(
        !ranges_utf8.is_empty() && !ranges_utf16.is_empty(),
        "expected diagnostics from non_ascii_broken under both encodings"
    );

    let char_utf8 = ranges_utf8[0].start.character;
    let char_utf16 = ranges_utf16[0].start.character;

    assert_ne!(
        char_utf8, char_utf16,
        "UTF-8 byte column ({char_utf8}) should differ from UTF-16 column ({char_utf16}) \
         because α precedes `nonexistent_column` on the same line; \
         if they are equal the encoding path is broken"
    );
    assert!(
        char_utf8 > char_utf16,
        "UTF-8 byte column ({char_utf8}) should be greater than UTF-16 column ({char_utf16}) \
         (α is 2 UTF-8 bytes but 1 UTF-16 code unit)"
    );
}
