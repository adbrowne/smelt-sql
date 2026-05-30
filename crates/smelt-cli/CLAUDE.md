# crates/smelt-cli/CLAUDE.md

Command-line interface — argument parsing, terminal output formatting, and surface adapters over `smelt-runtime` (for run/build/backfill) and `smelt-db` (for diagnostics, type checks, completions). The `smelt` binary lives here.

## How to test

```bash
# Unit tests
cargo test -p smelt-cli

# Example diagnostics gate (Salsa-direct path — fast)
cargo test -p smelt-cli --test example_diagnostics

# Full integration suite (builds and runs example workspaces against DuckDB)
cargo test -p smelt-cli --test ecommerce_execution
cargo test -p smelt-cli --test web_analytics_incremental_classification
# ... (tests/ has 50+ integration files)
```

`tests/example_diagnostics.rs` is the standing gate for workspace loading parity via the Salsa-direct path. Pair it with `cargo test -p smelt-lsp --test example_workspaces` (the LSP path) when touching discovery or `smelt-core::workspace`.

## Gotchas

- **`commands/run.rs` calls `smelt_runtime::execute_project`** — it contributes only argument parsing and a `RunReporter` impl. Do not add compile or execute logic to `commands/run.rs`. See root `CLAUDE.md` §Architectural invariants — **Run Pipeline Parity** and **Workspace Loading Parity** are load-bearing for work in this crate.
- **`diagnostics_terminal.rs` is the CLI's diagnostic boundary converter.** It wraps `line_index::LineIndex` and converts `rowan::TextRange` to `(line, col)` for terminal output. This is the CLI-side counterpart of `smelt-lsp::diagnostics_boundary::BoundaryConverter`.
- **`lib.rs` re-exports many symbols** that were historically part of the CLI's own compiler. Most of these now delegate to `smelt-runtime` or `smelt-core`. Check whether a symbol you need already exists upstream before adding it here.
- **`tests/incremental/`** is a directory of incremental model integration tests, not a single file.
- **DuckDB system library required for most integration tests.** See root `CLAUDE.md` §"Build and Test (System DuckDB)" for setup.

## Where things live

- `src/commands/` — one file per subcommand (`run.rs`, `build.rs`, `test.rs`, `explain.rs`, etc.)
- `src/diagnostics_terminal.rs` — `TerminalConverter`; CLI-side `TextRange` → `(line, col)`
- `src/compiler.rs` — thin re-exports from `smelt-runtime` (historical; delegates upstream)
- `src/executor.rs` — execution helpers used by commands
- `src/backfill.rs` — backfill plan computation
- `tests/` — 50+ integration test files, including `example_diagnostics.rs` (CI gate)
