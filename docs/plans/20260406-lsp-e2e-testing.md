# Plan: LSP End-to-End Testing

**Date**: 2026-04-06
**Research**: docs/research/2026-04-06-lsp-e2e-testing.md
**Status**: Validated

## Overview

Build an end-to-end test harness that exercises the full LSP protocol layer (tower-lsp + Backend) using in-process DuplexStream pairs. This catches bugs in the gap between "Salsa queries return correct data" and "VSCode receives a valid response" — the exact gap where three bugs were found on 2026-04-06 (overlapping TextEdit ranges, missing upstream column propagation, stale diagnostics after model rename).

## Current State

- `smelt-lsp` is a binary-only crate — `Backend` is private to `src/main.rs` (`main.rs:685-704`)
- All 116 tests in `tests/integration.rs` import from `smelt-db`/`smelt-parser` directly, never constructing a `Backend` or sending LSP messages
- `main()` at `main.rs:4042-4049` creates `LspService::new(Backend::new)` and serves over stdin/stdout
- tower-lsp 0.20 supports `DuplexStream`-based in-process testing (proven by ast-grep)

## Desired End State

- `smelt-lsp` has both a `lib.rs` (exports `Backend`) and a thin `main.rs`
- A new `tests/e2e.rs` file contains protocol-level tests using a `TestClient` harness
- Tests cover: initialize handshake, diagnostics on open, rename (CTE, model, column), goto-definition, and the three specific bug regression scenarios
- Existing `tests/integration.rs` is unchanged — DB-level tests remain as-is

## What We're NOT Doing

- Not rewriting existing integration tests — they're valuable at the DB layer
- Not testing the VSCode extension TypeScript code
- Not testing Python model discovery (requires Python runtime)
- Not building a general-purpose LSP test framework — just enough helpers for our needs
- Not testing completion or hover in this plan (add later)

## Implementation Phases

### Phase 1: Extract Backend into lib.rs

Split `main.rs` into `lib.rs` + `main.rs` so the E2E test can import `Backend`.

**Files to modify**:
- `crates/smelt-lsp/src/lib.rs` — new file, contains everything from `main.rs` except `fn main()`
- `crates/smelt-lsp/src/main.rs` — reduce to thin wrapper: `use smelt_lsp::Backend; fn main() { ... }`
- `crates/smelt-lsp/Cargo.toml` — add `[lib]` section alongside existing `[[bin]]`

**Changes**:
1. Create `src/lib.rs` that contains all of `main.rs` except `#[tokio::main] async fn main()` and the `#[cfg(test)] mod tests` block at the bottom. Make `Backend` and its `new` method `pub`.
2. Reduce `src/main.rs` to:
   ```rust
   use tower_lsp::{LspService, Server};
   use smelt_lsp::Backend;
   
   #[tokio::main]
   async fn main() {
       let stdin = tokio::io::stdin();
       let stdout = tokio::io::stdout();
       let (service, socket) = LspService::new(Backend::new);
       Server::new(stdin, stdout, socket).serve(service).await;
   }
   ```
3. Move the `#[cfg(test)] mod tests` block from `main.rs` into `lib.rs` (these test private helper functions like `determine_completion_context` that live in lib.rs).
4. Add to `Cargo.toml`:
   ```toml
   [lib]
   name = "smelt_lsp"
   path = "src/lib.rs"
   ```
5. Ensure `Backend::new` signature matches what `LspService::new` expects: `fn new(client: Client) -> Self`. Make `Backend` and `new` pub, keep everything else private.

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-lsp` (all 116 existing tests + unit tests pass)
- [x] `cargo build -p smelt-lsp` (binary still builds)
- [x] The `smelt-lsp` binary works: `echo '' | timeout 2 ./target/debug/smelt-lsp 2>/dev/null || true` (exits without panic)

### Phase 2: Test Harness (TestClient)

Build the DuplexStream-based test client in a new test file.

**Files to modify**:
- `crates/smelt-lsp/tests/e2e.rs` — new file
- `crates/smelt-lsp/Cargo.toml` — add dev-dependencies

**Changes**:
1. Add dev-dependencies to `Cargo.toml`:
   ```toml
   [dev-dependencies]
   tempfile = "3"
   tokio = { version = "1", features = ["full", "test-util"] }
   serde_json = "1.0"
   tower-lsp = { version = "0.20" }
   lsp-types = "0.94"
   ```

2. Create `tests/e2e.rs` with a `TestClient` struct that encapsulates the DuplexStream pattern:

   **TestClient** should provide:
   - `new(workspace_dir: &Path) -> Self` — creates `LspService::new(Backend::new)`, DuplexStream pairs, spawns the server task, sends `initialize` (with workspace_dir as the workspace folder) and `initialized`, waits for the initialize response
   - `send_request(method: &str, params: serde_json::Value) -> serde_json::Value` — sends a JSON-RPC request with auto-incrementing ID, waits for the response (collecting and buffering any notifications that arrive before the response)
   - `send_notification(method: &str, params: serde_json::Value)` — sends a JSON-RPC notification (no response expected)
   - `open_file(uri: &str, text: &str)` — sends `textDocument/didOpen`
   - `collect_diagnostics(timeout_ms: u64) -> Vec<(String, Vec<lsp_types::Diagnostic>)>` — reads notifications until timeout, returns all `publishDiagnostics` notifications as (uri, diagnostics) pairs
   - `shutdown()` — sends `shutdown` + `exit`

   **Protocol helpers** (private functions in the same file):
   - `encode_message(msg: &serde_json::Value) -> Vec<u8>` — wraps JSON with `Content-Length: N\r\n\r\n`
   - `read_message(stream: &mut DuplexStream) -> Option<serde_json::Value>` — reads one Content-Length framed message, returns None on EOF
   - `read_message_timeout(stream, timeout) -> Option<serde_json::Value>` — reads with timeout using `tokio::time::timeout`

   **TestWorkspaceDir** helper (reusable filesystem setup):
   - `new() -> Self` — creates a temp dir with a `models/` subdirectory
   - `add_model(name: &str, sql: &str)` — writes `models/{name}.sql`
   - `set_sources_yml(content: &str)` — writes `sources.yml` at root
   - `path() -> &Path`
   - `model_uri(name: &str) -> String` — returns `file:///tmp/.../models/{name}.sql`

3. Write one smoke test to verify the harness works:
   ```rust
   #[tokio::test]
   async fn test_initialize_and_diagnostics() {
       let ws = TestWorkspaceDir::new();
       ws.add_model("users", "SELECT id, name FROM smelt.ref('missing')");
       let mut client = TestClient::new(ws.path()).await;
       let diags = client.collect_diagnostics(2000).await;
       // Should have a diagnostic for undefined ref 'missing'
       assert!(!diags.is_empty());
       client.shutdown().await;
   }
   ```

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-lsp` (all existing tests + new smoke test pass)
- [x] The smoke test actually exercises the LSP protocol (not just DB queries)

### Phase 3: Rename E2E Tests

Test the rename flow end-to-end, covering the three bugs found on 2026-04-06.

**Files to modify**:
- `crates/smelt-lsp/tests/e2e.rs` — add rename tests

**Changes**:
1. Add helper method to `TestClient`:
   - `rename(uri: &str, line: u32, col: u32, new_name: &str) -> serde_json::Value` — sends `textDocument/rename`, returns the WorkspaceEdit response
   - `prepare_rename(uri: &str, line: u32, col: u32) -> serde_json::Value` — sends `textDocument/prepareRename`

2. Add helper to validate a WorkspaceEdit:
   - `assert_no_overlapping_edits(edit: &serde_json::Value)` — checks that no two TextEdits in the same document have overlapping ranges. This catches bug #1 (overlapping edits).

3. **Regression test: overlapping edits on multiline SELECT** (bug #1)
   ```
   Setup: events.sql with multiline SELECT ending in bare "properties" before FROM
   Action: rename "properties" at the correct position
   Assert: WorkspaceEdit has no overlapping ranges
   Assert: all edit ranges are within document bounds
   ```

4. **Regression test: qualified column propagates upstream** (bug #2)
   ```
   Setup: events.sql (defines "properties"), event_properties.sql (uses e.properties ->> 'x')
   Action: rename "properties" from event_properties.sql
   Assert: WorkspaceEdit contains edits in BOTH files
   Assert: events.sql edit targets the "properties" column definition
   ```

5. **Regression test: no stale diagnostics after model rename** (bug #3)
   ```
   Setup: upstream.sql, downstream.sql (refs upstream)
   Action: rename the model ref "upstream" → "new_name" from downstream.sql
   Assert: WorkspaceEdit includes file rename + ref text edits
   Then: simulate VSCode behavior — send didOpen for new file, didChange for edited files
   Then: collect diagnostics
   Assert: no "undefined model reference" diagnostics for "new_name"
   ```

6. **Basic CTE rename test**
   ```
   Setup: model with WITH cte AS (...) SELECT ... FROM cte
   Action: rename "cte" → "renamed_cte"
   Assert: WorkspaceEdit edits both definition and reference
   Assert: no overlapping ranges
   ```

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-lsp --test e2e` (all E2E tests pass)
- [x] All three regression tests would have caught the bugs found on 2026-04-06

### Phase 4: Goto-Definition and Code Action E2E Tests

Test the request/response cycle for goto-definition and code actions.

**Files to modify**:
- `crates/smelt-lsp/tests/e2e.rs` — add more tests

**Changes**:
1. Add helper:
   - `goto_definition(uri: &str, line: u32, col: u32) -> serde_json::Value` — sends `textDocument/definition`
   - `code_actions(uri: &str, line: u32, col: u32) -> serde_json::Value` — sends `textDocument/codeAction` with the diagnostic range

2. **Goto-definition for smelt.ref()**
   ```
   Setup: downstream.sql with smelt.ref('upstream'), upstream.sql
   Action: goto-definition on 'upstream' in the ref call
   Assert: response points to upstream.sql, line 0
   ```

3. **Goto-definition for column through ref**
   ```
   Setup: upstream.sql (SELECT id, name), downstream.sql (SELECT u.id FROM smelt.ref('upstream') u)
   Action: goto-definition on "id" in downstream
   Assert: response points to upstream.sql at the id column definition
   ```

4. **Code action: create missing model**
   ```
   Setup: model.sql with smelt.ref('nonexistent')
   Action: collect diagnostics, request code actions for the undefined-ref diagnostic
   Assert: code action with title containing "Create" is offered
   ```

5. **Diagnostics clear after fixing error**
   ```
   Setup: model.sql with smelt.ref('missing')
   Assert: diagnostics show undefined ref
   Action: add missing.sql via didOpen
   Then: send didChange for model.sql (same content, triggers re-diagnosis)
   Assert: diagnostics are now empty for model.sql
   ```

**Verification**:
- [x] `cargo fmt --all -- --check`
- [x] `cargo clippy --all-targets` (no warnings)
- [x] `cargo test -p smelt-lsp --test e2e` (all E2E tests pass)
- [x] `cargo test -p smelt-lsp` (all tests pass — both integration.rs and e2e.rs)

## Testing Strategy

Run the full E2E suite with:
```bash
cargo test -p smelt-lsp --test e2e
```

The E2E tests complement (not replace) the existing DB-level tests:
- **integration.rs** (116 tests): Fast, tests query logic directly
- **e2e.rs** (new): Slower, tests the full protocol layer including state management

Both should pass in CI. E2E tests will be slower due to tokio runtime + async I/O, but should complete in under 10 seconds total.

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| DuplexStream buffer too small for large WorkspaceEdits | Use `duplex(64 * 1024)` (64KB) — generous for JSON responses |
| Diagnostic notifications arrive out of order or race with responses | `collect_diagnostics` uses timeout-based polling; `send_request` buffers notifications while waiting for response |
| `Backend` depends on filesystem for initialize (reads model files) | `TestWorkspaceDir` creates real temp dirs with real files, same as existing `TestWorkspace` |
| Extracting lib.rs changes pub/private boundaries | Only make `Backend` and `Backend::new` pub — everything else stays private. Integration tests don't import from the crate. |
| Flaky tests from timing | Use generous timeouts (2-5 seconds) for diagnostic collection; deterministic assertions on response content |
