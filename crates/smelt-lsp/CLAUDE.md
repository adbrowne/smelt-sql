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

`tests/` contains 15+ integration test files. `example_workspaces` is the standing CI gate for workspace loading parity; `position_encoding` is the gate for diagnostic range encoding. Run both when touching LSP startup, workspace discovery, or `diagnostics_boundary`.

## Gotchas

- **`diagnostics_boundary.rs`** is the only place `rowan::TextRange` → `lsp_types::Range` conversion happens. `BoundaryConverter` is constructed once per file at the analysis/protocol boundary and must consult the negotiated `positionEncodingKind`. See root `CLAUDE.md` §Architectural invariants — **Diagnostic Range Encoding** and **Workspace Loading Parity** are load-bearing here.
- **`Backend::initialize` must call `smelt_core::workspace::load_workspace`** for every project it discovers — not walk the filesystem itself. LSP-only filesystem walking is the failure mode that caused the `functions/` discovery miss and the `set_loader_file` miss.
- **`backend.rs` is large.** The `LanguageServer` trait impl dispatches to per-feature helpers in `hover.rs`, `completion.rs`, `column_resolution.rs`. Read the dispatch surface in `backend.rs` first; drill into helpers only for the feature you're changing.
- **`hover.rs` re-exports** many helpers for integration tests — they're `pub` by design even if they look internal.
- **`python_scan.rs`** handles Python model scanning with caching; it's separate from the Salsa pipeline.

## Where things live

- `src/backend.rs` — `Backend` struct, `LanguageServer` trait impl, dispatch logic
- `src/diagnostics_boundary.rs` — `BoundaryConverter`; UTF-8/UTF-16 encoding-aware range converter
- `src/hover.rs` — hover text formatters, goto-def helpers, completion formatters (many re-exported)
- `src/completion.rs` — completion context detection (`determine_completion_context`)
- `src/column_resolution.rs` — column tracing for goto-def and hover
- `src/db_helpers.rs` — thin path→Salsa input lookups
- `tests/` — integration tests; `example_workspaces.rs` and `position_encoding.rs` are CI gates
