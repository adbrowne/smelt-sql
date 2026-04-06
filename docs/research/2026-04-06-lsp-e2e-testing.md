# Research: LSP End-to-End Testing via tower-lsp

**Date**: 2026-04-06
**Topic**: How to build end-to-end tests that exercise the full LSP protocol layer (not just Salsa queries)
**Branch**: main
**Commit**: 3eeb998

## Summary

All current LSP tests operate at the Salsa database level, completely bypassing the tower-lsp protocol layer. Three bugs found in production (overlapping TextEdit ranges, missing upstream propagation, stale Salsa state after file rename) were all in the gap between "correct query results" and "correct LSP responses". The tower-lsp crate supports in-process testing via `DuplexStream` pairs — no subprocess needed. The ast-grep project provides a proven reference implementation of this pattern.

## Key Files

| File | Purpose | Key Lines |
|------|---------|-----------|
| `crates/smelt-lsp/src/main.rs` | LSP server implementation | L4042-4049 (main/stdio), L685-704 (Backend struct), L1285-1544 (initialize) |
| `crates/smelt-lsp/tests/integration.rs` | Current test suite (116 tests, all DB-level) | L59-65 (TestWorkspace), L69-86 (new), L93-106 (add_model) |
| `crates/smelt-cli/tests/example_diagnostics.rs` | Diagnostic smoke tests against example workspaces | L1-94 |

## Architecture & Data Flow

### Server startup (stdio)

```
main() → tokio::io::stdin/stdout → Server::new(stdin, stdout, socket).serve(service)
```

`LspService::new(Backend::new)` creates the service. `Backend::new` takes a `Client` handle for sending notifications (diagnostics, log messages). The `Server` handles Content-Length framing and JSON-RPC dispatch automatically.

### Initialize handshake

1. Client sends `initialize` with `workspace_folders`
2. Server scans each workspace folder via `find_smelt_projects()` → discovers `models/` dirs
3. For each `.sql` file: reads content, calls `register_sql_content()`, adds to `all_files`
4. For each project root: loads `sources.yml`, loads `smelt.yml` config
5. Python models: discovers `.py` files, executes them, registers virtual `.sql` paths
6. Returns `InitializeResult` with capabilities (textDocumentSync=FULL, rename, goto-def, etc.)
7. Client sends `initialized` notification
8. Server registers dynamic file watchers for `.py` files

### Document lifecycle

- `did_open`: Registers file in Salsa DB, adds to `all_files` if new, publishes diagnostics
- `did_change`: Updates file text in Salsa DB (FULL sync — entire document), publishes diagnostics
- `did_change_watched_files`: Handles `.py` and `sources.yml` changes from disk

### Diagnostics flow

`publish_diagnostics(uri)` → `db.file_diagnostics(path)` + `db.type_diagnostics(path)` → converts to LSP `Diagnostic` → `client.publish_diagnostics()`. Diagnostics are pushed as notifications (server → client), not pulled.

`publish_all_diagnostics()` iterates all known files and publishes for each.

### Rename flow (the area with most bugs)

1. `prepare_rename` → `symbol_at_cursor()` → returns range + placeholder
2. `rename` → builds `WorkspaceEdit` with text edits + optional `RenameFile` operation
3. For model rename: pre-updates Salsa DB (all_files, file_text) before returning edit
4. VSCode applies the edit, sends `did_change` for modified files, `did_open` for renamed file

## Current Test Coverage

### What's tested (116 tests in integration.rs)

All tests use `TestWorkspace` which directly creates a `Database` and calls Salsa queries. Covers:
- Diagnostics (11 tests): parse errors, undefined refs, type mismatches
- Goto-definition (19 tests): refs, sources, columns, CTEs, wildcards
- Hover (5 tests): schema display, type inference
- Completion (11 tests): model names, CTE names, column names
- Find references (8 tests): model refs, CTE refs, source refs
- Code actions (14 tests): create model, add source, extract/inline CTE
- Rename (24 tests): CTEs, models, sources, columns
- Incremental updates (4 tests): file changes, diagnostic refresh

### What's NOT tested

- **LSP protocol framing**: Content-Length headers, JSON-RPC envelope
- **WorkspaceEdit validity**: Overlapping ranges, correct URIs, version fields
- **Notification ordering**: Diagnostics arriving after edits are applied
- **State transitions**: File renames updating `all_files`, `did_open`/`did_change` sequencing
- **Initialize handshake**: Workspace scanning, capability negotiation
- **Multi-step interactions**: rename → diagnostics refresh → no stale errors

## In-Process Testing Pattern (from ast-grep)

The ast-grep project tests tower-lsp servers using `DuplexStream` pairs — no subprocess:

```rust
use tokio::io::DuplexStream;
use tower_lsp::{LspService, Server};

fn create_lsp() -> (DuplexStream, DuplexStream) {
    let (service, socket) = LspService::new(|client| Backend::new(client));
    let (req_client, req_server) = tokio::io::duplex(1024);
    let (resp_server, resp_client) = tokio::io::duplex(1024);
    tokio::spawn(Server::new(req_server, resp_server, socket).serve(service));
    (req_client, resp_client)
}
```

### Key helpers needed

1. **`req(msg: &str) -> Vec<u8>`** — wraps JSON-RPC message with `Content-Length: N\r\n\r\n` header
2. **`parse_response(bytes: &[u8]) -> Vec<serde_json::Value>`** — parses Content-Length framed responses
3. **`wait_for_diagnostics(stream, uri)`** — reads notifications until matching `publishDiagnostics` arrives
4. **`wait_for_response(stream, id)`** — reads until response with matching ID arrives

### What ast-grep tests cover

- Initialize/initialized handshake
- `textDocument/didOpen` → diagnostics published
- Code action request → response → apply edits
- File watching registration
- Multiple overlapping edits

### Adaptation for smelt

The smelt server needs a real filesystem for `initialize` (it scans `models/` directories). Tests would:
1. Create a temp directory with `models/*.sql` and optional `sources.yml`
2. Create the LSP service with `Backend::new`
3. Send `initialize` with `workspace_folders` pointing to the temp dir
4. Send `initialized`
5. Wait for diagnostics (or send `textDocument/didOpen`)
6. Exercise features: rename, goto-def, code actions
7. Assert on responses and subsequent diagnostics

This is similar to the existing `TestWorkspace` pattern but communicating via LSP protocol instead of direct DB calls.

## Related Patterns

- `TestWorkspace` in `integration.rs` already creates temp dirs with model files — the filesystem setup can be reused
- `example_diagnostics.rs` in `smelt-cli` tests diagnostics against real example workspaces — same approach works for E2E tests
- The `register_sql_content` method (`main.rs:1123`) handles multi-model files — E2E tests should cover this

## Open Questions

1. **Buffer size**: ast-grep uses `duplex(1024)` — may need larger buffers for responses with many edits (rename across many files). Need to test or use a larger default.
2. **Timing**: Diagnostics are async notifications. Need a robust polling mechanism (read with timeout) to wait for them without hanging tests.
3. **Python models**: E2E tests that exercise Python model scanning need Python installed. Should these be behind a feature flag or separate test target?
4. **Workspace scanning depth**: `find_smelt_projects()` recurses into subdirectories. Need to understand if test temp dirs need a `models/` subdirectory or if any structure works.
