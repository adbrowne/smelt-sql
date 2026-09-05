# crates/smelt-lsp/CLAUDE.md

LSP server implementation — `Backend` struct (tower-lsp `LanguageServer` impl), diagnostics boundary conversion, completions, hover, goto-definition, and rename.

## How to test

```bash
# Unit tests (inline #[cfg(test)] in src/tests.rs)
cargo test -p smelt-lsp

# Integration test suite (drives the real Backend against example workspaces)
cargo test -p smelt-lsp --test example_workspaces

# Position encoding gate (UTF-8 and UTF-16, including non-ASCII)
cargo test -p smelt-lsp --test position_encoding
```

`tests/` contains 15+ integration test files. `example_workspaces` is the standing CI gate for workspace loading parity; `position_encoding` is the gate for diagnostic range encoding; `property_diff_parity` is the standing gate for property-diff surface parity (code lens + `PropertyDowngrade` diagnostics vs the CLI's `DiffReport`). Run all three when touching LSP startup, workspace discovery, `diagnostics_boundary`, or `property_diff`.

## Gotchas

- **`diagnostics_boundary.rs`** is the only place `rowan::TextRange` → `lsp_types::Range` conversion happens. `BoundaryConverter` is constructed once per file at the analysis/protocol boundary and must consult the negotiated `positionEncodingKind`. See root `CLAUDE.md` §Architectural invariants — **Diagnostic Range Encoding** and **Workspace Loading Parity** are load-bearing here.
- **`Backend::initialize` must call `smelt_core::workspace::load_workspace`** for every project it discovers — not walk the filesystem itself. LSP-only filesystem walking is the failure mode that caused the `functions/` discovery miss and the `set_loader_file` miss.
- **`backend.rs` is large.** The `LanguageServer` trait impl dispatches to per-feature helpers in `hover.rs`, `completion.rs`, `column_resolution.rs`. Read the dispatch surface in `backend.rs` first; drill into helpers only for the feature you're changing.
- **`hover.rs` re-exports** many helpers for integration tests — they're `pub` by design even if they look internal.
- **`python_scan.rs`** handles Python model scanning with caching; it's separate from the Salsa pipeline.
- **`property_diff_coalescing.rs` drives `did_change_watched_files` directly on `Backend`, not over the duplex-stream wire** — its file comment attributes this to a transport artifact ("only the first `FileEvent` of a multi-event burst reaches the handler over the wire") found while writing the test. Re-investigated for `docs/outcomes/20260905-property-diff/phases/08-plan.md` task 6: two direct probes — a minimal `initialize`/`initialized`/`didChangeWatchedFiles` round trip, and one replaying `property_diff_coalescing.rs`'s own scenario (a staged `examples/timeseries` git repo, 10 alternating `.git/HEAD` / `.git/refs/heads/main` `FileEvent`s in one notification) — both sent over the real `tower_lsp::Server::serve` + `tokio::io::duplex` wire used elsewhere in this crate's tests, dumping the encoded wire body's event count against `params.changes.len()` logged inside the handler. Both probes delivered **all 10** events to the handler; the claimed truncation did not reproduce. Verdict: **not a real transport defect** — no user-visible correctness bug, no spec Known Divergence warranted. The original observation was most likely a one-off artifact of the ad hoc, uncommitted harness used while first writing the test (never checked in, so not independently reproducible from history). The test's own choice to call `did_change_watched_files` directly stands regardless — it is the simpler, more deterministic way to isolate the coalescing logic from wire framing — but the file comment's transport-defect claim should not be repeated or extended.
- **`property_diff.rs` depends on `smelt-runtime`/`smelt-logical`** — the one exception to "LSP needs stop at the analysis layer" (see `smelt-runtime/CLAUDE.md`). `refresh_property_diff` in `backend.rs` MUST stay off the request path (`spawn_blocking`); never call `crate::property_diff::refresh` directly from a request handler (`code_lens`, `hover`, etc.) — those only read `Backend::property_diff`'s cached `ProjectDiffState`. Any `client.register_capability(...).await` (or other server→client request) blocks forever against a test harness that never answers server-initiated requests, so anything that must run even when dynamic registration never completes (like the initial property-diff refresh) goes BEFORE that call in `initialized`, not after.

## Where things live

- `src/backend.rs` — `Backend` struct, `LanguageServer` trait impl, dispatch logic
- `src/diagnostics_boundary.rs` — `BoundaryConverter`; UTF-8/UTF-16 encoding-aware range converter
- `src/hover.rs` — hover text formatters, goto-def helpers, completion formatters (many re-exported)
- `src/completion.rs` — completion context detection (`determine_completion_context`)
- `src/column_resolution.rs` — column tracing for goto-def and hover
- `src/db_helpers.rs` — thin path→Salsa input lookups
- `src/property_diff.rs` — property-diff editor integration: `ProjectDiffState`, `anchor_for`, `diagnostics_for_model`, `refresh` (the pipeline entry point `Backend::refresh_property_diff` runs in `spawn_blocking`)
- `tests/` — integration tests; `example_workspaces.rs`, `position_encoding.rs`, and `property_diff_parity.rs` are CI gates
