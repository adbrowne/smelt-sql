# crates/smelt-db/CLAUDE.md

Salsa incremental compilation database — wraps `smelt-parser` in cached queries and owns all type inference, schema extraction, diagnostic production, and workspace ingestion.

## How to test

```bash
# Full unit + integration test suite (includes property tests)
cargo test -p smelt-db

# Run only property-based type inference tests (slow; bump cases for deeper coverage)
cargo test -p smelt-db --test type_property_tests prop_type_inference
PROPTEST_CASES=1000 cargo test -p smelt-db --test type_property_tests prop_type_inference
```

The `tests/` directory contains 50+ integration test files. `prop_helpers/` contains the DuckDB oracle, type generators, and divergence registry used by the property tests.

## Gotchas

- **Salsa macro expansions are large.** Never `cat` `src/lib.rs` or any query file whole — `rg` into specific symbol names. The `#[salsa::tracked]` attribute expands to hundreds of lines per function.
- **Pure functions, thin Salsa wrappers.** Analysis logic (type inference, schema extraction, diagnostic checks) lives as plain Rust functions in `src/type_inference/` and `src/schema.rs`. Salsa queries in `src/queries/` gather inputs and call those functions; they do not contain logic themselves. See root `CLAUDE.md` §Architectural invariants — **Salsa Purity** and **Project Isolation** are the load-bearing invariants for work here.
- **`workspace_ingest.rs` is the Salsa-side of workspace loading.** The sequence `set_project_input → set_source_file → register_loader_files_from_disk` is centralised here; callers (`smelt-core::workspace::load_workspace`) call it after disk discovery.
- **`type_inference/` is split into sub-modules** by expression kind: `binary`, `literal`, `function_call`, `composite`, `hof`, `subquery`, `ternary`, `record`, `multi_model`, `values`, `loader_and_reflection`, `dispatch` (the orchestrator), `type_context` (shared context struct). Add new type rules in the appropriate sub-module.
- **`diagnostics_types.rs`** is the shared `Diagnostic` struct (carries `rowan::TextRange`, not line/col). Never add `lsp_types::Range` or `(line, col)` fields to it — conversion happens at the LSP and CLI boundaries.

## Where things live

- `src/queries/` — per-feature Salsa tracked functions (parse, schema, check_types, functions, project, loader)
- `src/type_inference/` — pure type inference, split by expression category
- `src/schema.rs` — `Column`, `ModelSchema` pure data types
- `src/workspace_ingest.rs` — Salsa-side workspace population (pair to `smelt-core::workspace`)
- `tests/prop_helpers/` — proptest oracle, generators, divergence registry
