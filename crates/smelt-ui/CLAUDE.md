# crates/smelt-ui/CLAUDE.md

Web UI server — Axum HTTP server embedding the compiled React frontend, WebSocket push for live updates, and a thin `RunManager` adapter over `smelt-runtime::execute_project`.

## How to test

```bash
cargo test -p smelt-ui
```

Integration-level UI tests drive the HTTP endpoints via `reqwest` or `axum::test`; most execution correctness is covered by `smelt-cli`'s integration tests (the execute_parity property).

## Gotchas

- **The frontend must be built before `smelt-ui` compiles.** `src/server.rs` embeds `ui/dist/` via `rust-embed`. Run `cd ui && npm run build` first if `ui/dist/` is missing or stale — otherwise the crate will compile but serve a 404 for every asset.
- **`run_manager.rs` is a surface adapter, not a reimplementation.** It converts HTTP `RunExecuteRequest` → `smelt_runtime::ExecuteRequest`, implements `RunReporter` over a WebSocket broadcast channel, and calls `execute_project`. No execute logic lives here. See root `CLAUDE.md` §Architectural invariants — **Run Pipeline Parity** is load-bearing here.
- **`build.rs` uses `line_index::LineIndex` directly** for the JSON serialization layer — diagnostic ranges arrive from `smelt-db` as `rowan::TextRange` and are converted at the HTTP boundary (same pattern as the LSP and CLI boundaries, just using a plain `LineIndex` rather than a named converter struct).
- **Spark is not supported in UI mode.** `RunManager`'s `BackendFactory` creates DuckDB only. A Spark path would need changes here.
- **`src/watcher.rs`** watches model files for changes and broadcasts `ChangeEvent::ModelsUpdated` to WebSocket clients.

## Where things live

- `src/run_manager.rs` — `RunManager` (surface adapter; `RunReporter` impl, `BackendFactory` impl)
- `src/server.rs` — Axum router, WebSocket handler, `AppState`, `rust-embed` asset serving
- `src/build.rs` — project/graph response builders; uses `LineIndex` for diagnostic ranges
- `src/api.rs` — REST endpoint handlers
- `src/watcher.rs` — filesystem watcher for live reload
- `../../ui/` — React frontend source (compiled into `ui/dist/`, embedded at build time)
