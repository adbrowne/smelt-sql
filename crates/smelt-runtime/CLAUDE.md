# crates/smelt-runtime/CLAUDE.md

Compile and execute driver — the single shared pipeline consumed by both `smelt-cli` and `smelt-ui`. Owns model selection/filtering, SQL compilation (function-body resolution, ephemeral inlining, type-cast wrapping, time-filter injection), the pre-execution diagnostic gate, and the per-model execute loop.

## How to test

```bash
cargo test -p smelt-runtime
```

Integration tests in `tests/compile_parity.rs` (SQL compilation equivalence across models), `tests/select_parity.rs` (selection/filtering logic), and `tests/execute_parity.rs` (dual-consumer fixture: same project through CLI-style and UI-style reporters, identical `RunOutcome`) live here. The `surface_audit` test verifies the `pub(crate)` structural enforcement clause.

## Gotchas

- **`execute_project(request, reporter)` is the single entry point.** Both `smelt-cli` and `smelt-ui` call it and contribute only surface adapters (`RunReporter` impls, argument parsing). Do not add a parallel compile or execute helper in either consumer. See root `CLAUDE.md` §Architectural invariants — **Run Pipeline Parity** is load-bearing for any work here.
- **`SqlCompiler` and its helpers (`EphemeralResolver`, `PrintContext` constructors) are `pub(crate)`** by design — consumers cannot construct a half-compiled model. If you need a new shape from the compiler, extend `smelt-runtime`, not the consumer crate.
- **`smelt-lsp` depends on this crate for exactly one thing: `property_diff`.** The property-diff editor integration (`docs/specs/property_diff.md` §Surface "Editor") needs `profile::profiles_for_workspace` and the `work_side`/`baseline_side`/`report` pipeline in `src/property_diff.rs`, so `smelt-lsp` added `smelt-runtime` + `smelt-logical` as normal dependencies (`docs/outcomes/20260905-property-diff/phases/07-plan.md` R1) — verified acyclic and DuckDB-free (`duckdb`/`smelt-backend-duckdb`/`smelt-backends` are this crate's `[dev-dependencies]` only). Do not add any OTHER LSP-serving logic here; LSP needs otherwise stop at the analysis layer (`smelt-db`, `smelt-core`).
- **`reporter.rs` defines the `RunReporter` trait** — implement it in consumer crates (`smelt-cli`'s terminal reporter, `smelt-ui`'s WebSocket reporter).
- **`transformer.rs`** owns `inject_time_filter` and `inject_source_filters`. Time-filter logic lives here, not in the CLI or UI.

## Where things live

- `src/execute.rs` — `execute_project`, `BackendFactory` trait, `BackendFuture`
- `src/compile.rs` — `SqlCompiler`, `CompiledModel`, `EphemeralResolver`, `UpstreamSchemas`
- `src/select.rs` — model selection/filtering, `SelectionRequest`, `SelectionPlan`
- `src/reporter.rs` — `RunReporter` trait, `NoOpReporter`
- `src/transformer.rs` — `inject_time_filter`, `inject_source_filters`, `TimeRange`
- `src/fn_bodies.rs` — function-body map construction (`build_fn_body_map`)
- `src/types.rs` — `ExecuteRequest`, `RunOutcome`
- `src/property_diff.rs` — the shared property-diff pipeline (`WorkSide`/`BaselineSide`/`work_side`/`baseline_side`/`report`), consumed by both `smelt-cli`'s `explain --diff` and `smelt-lsp`'s editor integration
