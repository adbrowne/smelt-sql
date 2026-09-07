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
- **The Python model path you test depends on the feature set.** `smelt-lsp` has
  `default = ["python"]`, which unifies `smelt-runtime/python` ON in any
  workspace-wide build. So `cargo test -p smelt-runtime --lib` exercises the
  subprocess path in `src/python.rs`, while `cargo test` / `mise run verify`
  exercises the embedded-PyO3 path in the same file — a bug in one is invisible
  to the other (issue #189). Reproduce the full-suite path with
  `cargo test -p smelt-runtime --lib --features python`. The embedded path shares
  one process-global interpreter across concurrent tests; its global state is
  serialised in `smelt-core`'s `python_models.rs`, not here.
- **`reporter.rs` defines the `RunReporter` trait** — implement it in consumer crates (`smelt-cli`'s terminal reporter, `smelt-ui`'s WebSocket reporter).
- **`transformer.rs`** owns `inject_time_filter` and `inject_source_filters`. Time-filter logic lives here, not in the CLI or UI.

## Windowed-keyed-maintenance driver module map

`src/maintenance_driver/` implements the mode-agnostic mechanism behind
`refresh: keyed`'s window-forward run shape (`docs/specs/model_transforms.md`
§Surface "Windowed-keyed-maintenance driver" and §Semantics "Keyed
`merge_into`"): classify → step over driving partitions in temporal order →
per-partition pushdown → create-or-merge. `keyed` is its first named consumer
(`WindowedKeyedRule` impl in `crate::cumulative`). Its submodules are pure
code organisation over that one mechanism:

- `driver` — the windowed-keyed loop itself: driving-partition stepping, the
  `WindowedKeyedRule` seam, and `run_windowed_keyed_maintenance`.
- `resolve` — plan-cell → live-technique resolution (creation strategy, fold
  deferral, column-scoped and in-place-update cells, horizon widening).
- `membership` — membership-sensitive recompute cells and their
  staged-candidate execution.
- `repair` — per-group recompute / repair cells and the diff-patch leg.
- `key_addressed` — key-addressed model-edge cells.
- `column_scoped` — column-scoped merge execution and the changed-row /
  changed-key predicates it dispatches on.
- `observed_delta` — reading an upstream driving model's observed delta back
  off the backend.
- `sidecar` — fingerprint and repair-group sidecar diffing and refresh.
- `delta_restriction` — delete+insert with a delta restriction, and the live
  facts that admit it.
- `succession` — the succession-patch technique's live-cell resolution and
  window-forward step loop (not a `driver::WindowedKeyedRule` impl — see the
  module's own doc comment for why).

## Where things live

- `src/execute.rs` — `execute_project`, `BackendFactory` trait, `BackendFuture`
- `src/compile.rs` — `SqlCompiler`, `CompiledModel`, `EphemeralResolver`, `UpstreamSchemas`
- `src/select.rs` — model selection/filtering, `SelectionRequest`, `SelectionPlan`
- `src/reporter.rs` — `RunReporter` trait, `NoOpReporter`
- `src/transformer.rs` — `inject_time_filter`, `inject_source_filters`, `TimeRange`
- `src/fn_bodies.rs` — function-body map construction (`build_fn_body_map`)
- `src/types.rs` — `ExecuteRequest`, `RunOutcome`
- `src/property_diff.rs` — the shared property-diff pipeline (`WorkSide`/`BaselineSide`/`work_side`/`baseline_side`/`report`), consumed by both `smelt-cli`'s `explain --diff` and `smelt-lsp`'s editor integration
