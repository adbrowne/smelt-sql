# Plan: `smelt-runtime` extraction and layered single-ownership rule

**Date**: 2026-05-23
**Spec**: [`docs/specs/architecture.md`](../specs/architecture.md)
**Spec diff**: adds a new §"Run pipeline parity rule (CLI ↔ UI)" alongside the existing two parity rules; adds `smelt-runtime` to §"Crate responsibilities"; adds a "Language-service slot (future)" entry under Known Divergences anticipating `smelt-language-service` extraction without committing to it; updates §"Compilation pipeline" to name `smelt-runtime` as the home of the compile+execute layer.
**Tracking branch**: a new branch off `worktree-web_analytics`, name TBD when work begins (suggested: `feat/smelt-runtime`).
**Docs**: code+docs (CLAUDE.md + spec updated alongside).
**Predecessor research**: [`docs/research/20260523-lsp-cli-ui-divergence.md`](../research/20260523-lsp-cli-ui-divergence.md).
**Motivating example**: today's panic at `crates/smelt-ui/src/run_manager.rs:508` ("Test models should not be executed directly") — the latest in a class of bugs where UI and CLI reimplement the model lifecycle differently. Verified divergences include UI skipping `smelt.fn.*` expansion (`compile_sql` passes `None` for `smelt_fn`), UI skipping `apply_type_casts`, UI missing the pre-execution `UnknownSmeltFn` gate, and UI not expanding `*.gen.sql` generators.

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/architecture.md` (the correctness oracle) and the predecessor research at `docs/research/20260523-lsp-cli-ui-divergence.md`.
2. Confirm you are on the tracking branch (not `worktree-web_analytics` directly — this plan does the work on a feature branch since it changes Cargo workspace structure).
3. Find the next `pending` phase in Progress tracking.

**For each phase:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**Conventions every phase:**
- Real-fixture tests in `examples/` for any behavior change (use `examples/web_analytics/` and `examples/functions_demo/` as the dual-consumer fixtures).
- Red-green TDD: failing test before any implementation. For "no behavior change" phases (1, 5), the test is that existing CI gates still pass; for behavior-change phases (2, 3, 4) a new fixture-driven test must exist before the move.
- Atomic per-phase commits using the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push the tracking branch.
- Honour `CLAUDE.md` invariants: `smelt-db` purity rule, workspace loading parity rule, project isolation rule. **None of these are weakened by this plan.** Analysis logic stays in `smelt-db`. The new `smelt-runtime` depends downward on `smelt-db` / `smelt-core` / `smelt-planner` / `smelt-backend` / `smelt-state`; it does not move logic up out of those crates.
- **Timeless-oracle rule.** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/architecture.md` and CLAUDE.md describe the rule as if it has always existed.
- **UI-editor-via-LSP forward compatibility.** Throughout extraction, anything that the LSP touches today (diagnostics, type inference, schema extraction, completions, hover, goto-def, workspace ingest) stays where it is. The new crate is for *execute-side* concerns only. If a future `smelt-language-service` extraction wants to share editor logic between LSP and UI, this plan must not have made that harder.

---

## Context

Today three consumers (`smelt-lsp`, `smelt-cli`, `smelt-ui`) sit on a clean analysis layer (`smelt-parser`, `smelt-db`, `smelt-core`, `smelt-planner`, `smelt-backend`) but diverge above it. The compile pipeline (`SqlCompiler`, `build_fn_body_map`, `apply_type_casts`, ephemeral inlining, `inject_time_filter`, function expansion) lives privately inside `smelt-cli`; `smelt-ui` reimplemented a thinner version with no function expansion, no type casts, and no ephemeral handling. The selection/filter pass (drop tests, drop `.gen` generators, expand emitted models, resolve selectors) lives in `smelt-cli` and was partially missing from `smelt-ui` (today's test-model panic). The execute loop (per-model batch dispatch, `MaterializationStrategy::Incremental`, manifest writes, interval-store updates) is implemented twice — `smelt-cli/src/commands/run.rs` (1404 lines) and `smelt-ui/src/run_manager.rs` (709 lines).

The fix is **layered single-ownership**: a new crate `smelt-runtime` owns the compile+execute layer; both `smelt-cli` and `smelt-ui` consume it via one entry point; consumer crates contribute only surface concerns (argument parsing, progress reporting, HTTP serialization). The LSP does *not* depend on `smelt-runtime` — its needs are met entirely by the existing analysis layer.

A future requirement is folded in: the UI may eventually expose an in-browser editor with diagnostics. The clean fit with this architecture is a future `smelt-language-service` crate that wraps `smelt-db`'s editor-relevant analysis behind a transport-agnostic API; both `smelt-lsp` (JSON-RPC adapter) and `smelt-ui` (HTTP/WS adapter) would depend on it. This plan **does not build** that crate, but it anticipates the slot in the spec so the eventual extraction is mechanical. The constraint enforced by this plan: **no editor-relevant analysis logic moves into `smelt-runtime`** — it must remain in `smelt-db` where the future `smelt-language-service` can reach it.

## Scope

### In scope (spec coverage)

- `architecture.md` §Surface "Crate responsibilities" — new row for `smelt-runtime`.
- `architecture.md` §Surface "Compilation pipeline" — name `smelt-runtime` as the owner of compile+execute.
- `architecture.md` §Semantics — new rule "Run pipeline parity rule (CLI ↔ UI)" alongside the existing Workspace Loading Parity Rule and Project Isolation Rule.
- `architecture.md` §Known Divergences — entry "Language-service slot (future)" anticipating `smelt-language-service` extraction.
- `CLAUDE.md` — one-line pointer to the new rule under the existing parity-rule section.
- New crate `smelt-runtime` owning:
  - `SqlCompiler`, `build_fn_body_map`, `build_fn_body_map_from_model_files`, `apply_type_casts`, ephemeral inlining, cross-engine ref resolution, `inject_time_filter`.
  - The selection/filter pass (resolve selectors, apply excludes, drop tests, drop `.gen` generators, expand emitted models).
  - The per-model execute loop (full refresh + incremental batches + cumulative dispatch).
  - The pre-execution diagnostic gate.
  - `RunReporter` trait + `ExecuteRequest` + `RunOutcome` plain types.
- `smelt-cli` and `smelt-ui` both switch to the shared entry point.
- New CI gate: a fixture-driven test that runs the same project through CLI and UI entry points and asserts identical model outputs, manifests, and selection sets.

### Explicitly deferred

- **`smelt-language-service` crate.** The spec records the slot under Known Divergences; the crate is not built here. Extraction will be triggered by the UI-editor feature work, not by this plan.
- **UI as LSP client.** Whether the UI eventually embeds an LSP client, talks to a sidecar `smelt-lsp` over WebSocket, or depends on `smelt-language-service` directly is a UI-editor-work design question, not a runtime-extraction question.
- **LSP consumption of `smelt-runtime`.** The LSP may in the future want dry-compile for richer diagnostics (type-cast warnings, ephemeral-resolution errors). The plan keeps `smelt-runtime::compile` callable as a pure function returning `CompiledModel` (no execute coupling) so this stays possible; it does not wire the LSP up to consume it.
- **Refactoring smelt-cli's CLI surface.** `smelt-cli/src/commands/` stays where it is. Only the helpers and execution loop move out.
- **State-writing refactor.** `smelt-state` (`RunManifest`, `IntervalStore`, `FileStore`) is already shared. No changes to that crate.
- **Generator pipeline extraction.** The emitted-models Salsa pipeline stays in `smelt-db`. `smelt-runtime` calls into it, not vice versa.
- **Spark backend reach.** `smelt-ui` today bails out on Spark targets (`run_manager.rs:706`). This plan does not fix that; once `smelt-runtime` owns execution, Spark works in the UI for free (or at least: the gap closes to the same gap the CLI has).
- **New diagnostic codes.** This plan moves the *gate* into `smelt-runtime`; the *codes* stay in `smelt-db`.
- **Run UI redesign / new endpoints.** Surface-layer changes are out of scope. The UI exposes the same `/api/run/execute` endpoint shape; only its internals change.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 0 — Spec update: `architecture.md` adds Run Pipeline Parity Rule, `smelt-runtime` crate row, language-service slot; CLAUDE.md pointer | done | 262abe65 | 2026-05-23 |
| 1 — Create `smelt-runtime` crate; move pure helpers (`inject_time_filter`, `build_fn_body_map*`); establish `RunReporter` trait | done | 270e705e | 2026-05-23 |
| 2 — Move `SqlCompiler` + emitters into `smelt-runtime::compile`; CLI + UI both consume; new compile-parity CI gate | done | e5407144 | 2026-05-23 |
| 3 — Move selection/filter pass into `smelt-runtime::select`; CLI + UI both consume; today's UI test-filter fix becomes the shared function | done | ad73f9b5 | 2026-05-23 |
| 4a — `smelt-runtime::execute_project` entry point + `BackendFactory` trait; UI shrinks to surface wrapper (726→317 lines) | done | 7599d648 | 2026-05-23 |
| 4b — CLI migrates to `execute_project`; cumulative + backfill + planner-orchestration move into runtime; end-to-end CLI↔UI parity CI gate | pending |  |  |
| 5 — Surface tightening: make `smelt-runtime` internals `pub(crate)`; delete consumer-side duplicates; final crate-graph audit | pending |  |  |

---

### Phase 0: Spec update — codify the Run Pipeline Parity Rule

**Goal.** Land the architectural rule in `docs/specs/architecture.md` and the pointer in `CLAUDE.md` *before* any crate work. The rule describes the layered architecture as if it has always existed (timeless-oracle); only this plan file carries the migration history.

**Pre-conditions.** None — entry point.

**TDD tests to write first.**
- None (documentation-only phase). The standing CI gates (`cargo test -p smelt-lsp --test example_workspaces`, `cargo test -p smelt-cli --test example_diagnostics`) must still pass — this phase changes no code.

**Implementation shape.**
- Update `docs/specs/architecture.md` §Surface "Crate responsibilities" table:
  - Add a row: `smelt-runtime` | async | "Compile + execute pipeline: `SqlCompiler`, fn-body resolution, ephemeral inlining, type casts, selection/filter, per-model batch loop, `RunReporter` trait. Consumed by `smelt-cli` and `smelt-ui`; not depended on by `smelt-lsp`."
- Update §Surface "Compilation pipeline" prose to name `smelt-runtime` as the home of the compile+execute layer; rewrite as if it has always lived there. The pipeline diagram currently in the spec stays accurate; the home moves.
- Add new §Semantics subsection **"Run pipeline parity rule (CLI ↔ UI)"**, structured like the existing Workspace Loading Parity Rule and Project Isolation Rule:
  - Statement: the compile+execute pipeline lives in exactly one place (`smelt-runtime`). Both `smelt-cli` and `smelt-ui` consume it via a single `execute_project(request, reporter)` entry point.
  - Rationale: incidents trace to two failure modes — (Mode A) a consumer reimplements analysis logic instead of calling the shared analysis layer (e.g. LSP `functions/` discovery miss — addressed by the existing Workspace Loading Parity Rule); (Mode B) a consumer reimplements compile/execute logic because there's no shared layer for it to call (e.g. UI test-model execution panic, UI `smelt.fn.*` non-expansion, UI missing `apply_type_casts`). Layered single-ownership closes both.
  - How to apply: classify a new piece of logic by who needs it. If the LSP needs it (parsing, analysis, type inference, schemas, diagnostics, workspace discovery, planning), it lives in `smelt-parser` / `smelt-db` / `smelt-core` / `smelt-planner` — not `smelt-runtime`. If only CLI and UI need it (compile pipeline, execute loop, manifests, selection/filter), it lives in `smelt-runtime`. Surface concerns (CLI args, HTTP, LSP RPC) live in the consumer crate.
  - Standing CI gate: a fixture-driven test (added in Phase 4) runs the same project through CLI and UI entry points and asserts identical model outputs, manifest contents, and selection sets. Fixtures must include test models (filter), generators (expansion), `smelt.fn.*` calls (compile), and cumulative aggregates (execute dispatch).
- Add new Known Divergences entry **"Language-service slot (future)"**: describe that editor-relevant analysis features (diagnostics, completions, hover, goto-def) currently live in `smelt-lsp` mixed with tower-lsp transport. A future `smelt-language-service` crate would extract the transport-agnostic portion so both `smelt-lsp` (JSON-RPC adapter) and `smelt-ui` (HTTP/WS adapter for in-browser editing) consume it. The Run Pipeline Parity Rule keeps this option open by forbidding analysis logic in `smelt-runtime`. Deferred until UI editing work begins.
- Update `CLAUDE.md`:
  - Under the existing "Workspace Loading Parity Rule" and "Project Isolation Rule" sections, add a third subsection **"Run Pipeline Parity Rule (CLI ↔ UI)"** with the same shape — a paragraph stating the rule, a "Why this matters" paragraph citing today's incident and the two prior LSP incidents as the same bug class at different layers, and a "The rule in practice" bulleted DO/DON'T list.
  - The CLAUDE.md prose should be terse — the spec is the canonical home; CLAUDE.md is a pointer with the "load-bearing tagline" so a fresh context sees the rule.

**Critical files (allowed to touch in this phase).**
- `docs/specs/architecture.md`
- `CLAUDE.md`

**Docs touched.**
- The above two — no user-facing docs change in this phase.

**Review checklist.**
- [ ] Rule statement is parallel in shape to the Workspace Loading Parity Rule and Project Isolation Rule (statement, rationale + incident catalogue, "in practice" DO/DON'T, standing CI gate).
- [ ] Spec prose is timeless — no "this plan adds…" or "Phase 4 will…". The reader cannot tell that the rule was recently added.
- [ ] The Known Divergences entry for `smelt-language-service` is *anticipatory*, not prescriptive — it documents the slot but does not commit to a shape.
- [ ] CLAUDE.md tagline mentions today's UI test-model incident explicitly; the spec rationale cites it too. Future engineers should be able to trace the rule back to the specific incident class.
- [ ] No code changes in this phase; all CI gates still green.

**Commit.** `docs(spec,claude): codify run pipeline parity rule (CLI ↔ UI)`

---

### Phase 1: Create `smelt-runtime` crate; move pure helpers

**Goal.** Establish the crate as a workspace member. Move the easy bits — pure helpers whose extraction has no behaviour consequence for either consumer. Define `RunReporter` trait + `ExecuteRequest` / `RunOutcome` plain types, but do not yet wire them through. After this phase, both `smelt-cli` and `smelt-ui` continue to use their existing pipelines unchanged; the new crate is a home for shared helpers and shapes.

**Pre-conditions.** Phase 0 done — the spec describes the layered architecture so reviewers can verify against it.

**TDD tests to write first.**
- `crates/smelt-runtime/src/lib.rs::tests::test_inject_time_filter_appends_to_where` — the existing UI behaviour (`run_manager.rs:644-676`) as a unit test against the new home.
- `crates/smelt-runtime/src/lib.rs::tests::test_inject_time_filter_creates_where_when_absent` — same.
- `crates/smelt-runtime/src/lib.rs::tests::test_inject_time_filter_bails_without_from` — same.
- `crates/smelt-runtime/src/lib.rs::tests::test_build_fn_body_map_from_model_files_extracts_define` — port the existing CLI test against the new home.
- `crates/smelt-runtime/src/lib.rs::tests::test_run_reporter_default_no_op` — the trait has a default no-op impl so consumers can opt out of progress (used by tests).
- Existing CI gates (`example_workspaces`, `example_diagnostics`, `cargo test -p smelt-cli`, `cargo test -p smelt-ui`) still pass.

**Implementation shape.**
- `crates/smelt-runtime/Cargo.toml` — new crate. Dependencies: `smelt-parser`, `smelt-core`, `smelt-db`, `smelt-planner`, `smelt-backend`, `smelt-state`, `smelt-dialect`, `anyhow`, `chrono`, `tokio`, `tracing`. Feature flags mirror `smelt-cli` for backend selection (`duckdb`, `spark`, `bundled-duckdb`).
- `crates/smelt-runtime/src/lib.rs` — crate root exposing the public surface (initially small).
- Move from `crates/smelt-ui/src/run_manager.rs:644-676` and `crates/smelt-cli/src/compiler.rs` (the equivalent helpers): `inject_time_filter`, `build_fn_body_map`, `build_fn_body_map_from_model_files`. Both consumers update their imports — no logic change.
- Add new types in `crates/smelt-runtime/src/types.rs`:
  - `pub struct ExecuteRequest { select: Vec<String>, exclude: Vec<String>, target: String, start: Option<String>, end: Option<String>, batch_size_days: Option<u32>, per_partition: bool, full_refresh: bool, dry_run: bool }` — a superset of today's CLI and UI request shapes.
  - `pub struct RunOutcome { run_id: String, models: HashMap<String, ModelRunRecord>, started_at: DateTime<Utc>, completed_at: Option<DateTime<Utc>>, total_rows: usize }`.
  - `pub trait RunReporter: Send + Sync + 'static { fn run_started(&self, _models: &[String], _total_batches: usize) {} fn model_started(&self, _name: &str, _idx: usize, _total: usize) {} fn batch_completed(&self, _model: &str, _idx: usize, _total: usize, _row_count: usize, _duration: Duration) {} fn model_completed(&self, _name: &str, _row_count: usize, _duration: Duration) {} fn run_failed(&self, _model: Option<&str>, _error: &str) {} fn run_cancelled(&self) {} }`.
  - `pub struct NoOpReporter;` implementing `RunReporter` with all defaults (for tests).
- The trait does **not** yet have an execute entry point — that lands in Phase 4. This phase just establishes the shape.
- `crates/smelt-cli/src/compiler.rs` and `crates/smelt-ui/src/run_manager.rs` switch their `inject_time_filter` / `build_fn_body_map` imports to `smelt_runtime::inject_time_filter` / `smelt_runtime::build_fn_body_map`. The CLI's existing helper re-export from `smelt-cli`'s `lib.rs` is kept for one transitional phase, then removed in Phase 5.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/Cargo.toml` (new).
- `crates/smelt-runtime/src/lib.rs` (new).
- `crates/smelt-runtime/src/types.rs` (new).
- `Cargo.toml` (workspace root) — add `smelt-runtime` to `[workspace.members]`.
- `crates/smelt-cli/Cargo.toml` — add `smelt-runtime` dependency.
- `crates/smelt-ui/Cargo.toml` — add `smelt-runtime` dependency.
- `crates/smelt-cli/src/compiler.rs` — delete the local `inject_time_filter` and `build_fn_body_map*` functions; switch imports.
- `crates/smelt-ui/src/run_manager.rs` — delete the local `inject_time_filter`; switch imports.
- `crates/smelt-cli/src/lib.rs` — temporary re-export of the moved helpers so existing internal callers compile.

**Docs touched.** None — Phase 0 already updated the spec to anticipate this crate.

**Review checklist.**
- [ ] `smelt-runtime` Cargo dependencies are *downward only* — no dependency on `smelt-cli` or `smelt-ui` or `smelt-lsp`.
- [ ] `RunReporter` trait has all-default-impl methods; `NoOpReporter` compiles. (Tests use it.)
- [ ] `ExecuteRequest` covers the union of today's CLI and UI request fields; no field is consumer-specific.
- [ ] Moved helpers have identical signatures to the originals (no semantic drift hidden under the move).
- [ ] `cargo build -p smelt-cli` and `cargo build -p smelt-ui` succeed unchanged.
- [ ] Existing CI gates green; no `cargo test` failures.

**Commit.** `feat(smelt-runtime): scaffold crate with pure helpers and RunReporter shape`

---

### Phase 2: Move `SqlCompiler` into `smelt-runtime::compile`; both consumers switch

**Goal.** This is the highest-impact phase — it closes the UI's silent compile divergences (no `smelt.fn.*` expansion, no `apply_type_casts`, no ephemeral inlining). Move `SqlCompiler` and its emitters out of `smelt-cli/src/compiler.rs` into `smelt-runtime::compile`. Make CLI and UI both consume the same entry point. Add a CI gate that asserts byte-identical SQL output from both consumers on the same fixture.

**Pre-conditions.** Phase 1 done.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/compile_parity.rs::test_compile_with_function_call` — a model using `smelt.functions.foo()` compiles identically in CLI and UI paths. (Today the UI returns the SQL unexpanded.)
- `crates/smelt-runtime/tests/compile_parity.rs::test_compile_with_ephemeral_dep` — a model depending on an ephemeral model gets the ephemeral CTE inlined. (Today the UI does not inline.)
- `crates/smelt-runtime/tests/compile_parity.rs::test_compile_applies_type_casts` — projections over `smelt.ref()` columns get CASTs applied per inferred types. (Today the UI does not.)
- `crates/smelt-runtime/tests/compile_parity.rs::test_compile_with_time_filter_injection` — incremental model with a time filter injected, then compiled, produces the same output via either entry point.
- `crates/smelt-runtime/tests/compile_parity.rs::test_compile_path_ref_resolution` — `smelt.<path>` references resolve to schema-qualified table names identically via both consumers.
- New integration test `crates/smelt-runtime/tests/compile_examples.rs` that walks every non-broken `examples/*/` project, compiles every non-test model via `smelt_runtime::compile`, and asserts no errors. Replaces the implicit coverage from `example_diagnostics`.

**Implementation shape.**
- New module `crates/smelt-runtime/src/compile.rs` (or split across `compile/mod.rs`, `compile/emitters.rs`, `compile/type_casts.rs`).
- Move the entire `SqlCompiler` impl, including:
  - The four emitter factories (`smelt_fn`, `smelt_as_struct`, `smelt_path_ref`, `smelt_path_call`).
  - `apply_type_casts` and the `wrap_with_type_casts` helper.
  - Ephemeral CTE inlining + cross-engine ref resolution.
  - The `compile_with_sql_and_ephemerals` entry point — rename to `smelt_runtime::compile(request: CompileRequest) -> Result<CompiledModel>` where `CompileRequest` bundles the model, schema, backend, fn-body-map, ephemerals, time-filter (optional).
- Make the entry point pure: no Salsa traits in its public signature. (The `db` + `workspace` consumers should call `build_fn_body_map` themselves and pass the resulting `FnBodyMap` in.) This preserves the smelt-db purity discipline and keeps the door open for future LSP dry-compile.
- `smelt-cli/src/compiler.rs` shrinks to: discover models, build inputs for `smelt_runtime::compile`, call it. The `commands/run.rs` path goes via the same shared compile call.
- `smelt-ui/src/run_manager.rs` deletes its local `compile_sql` (lines 627-642) and `PrintContext`-with-all-Nones; switches to `smelt_runtime::compile`.
- The pre-execution diagnostic gate (CLI's `commands/run.rs:643-656` checking `UnknownSmeltFn`) moves into `smelt_runtime::compile` as `gate_diagnostics(&db, workspace, &models) -> Result<()>` — *called by the compile entry point but separable* so callers can gate or not. Default behaviour: gate on the same code set as today's CLI.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/compile.rs` (new) or `crates/smelt-runtime/src/compile/` (new directory).
- `crates/smelt-runtime/src/lib.rs` — re-export the entry point.
- `crates/smelt-runtime/tests/compile_parity.rs` (new).
- `crates/smelt-runtime/tests/compile_examples.rs` (new).
- `crates/smelt-cli/src/compiler.rs` — gut the moved functions; keep the model-discovery scaffolding.
- `crates/smelt-cli/src/commands/run.rs` — switch to the shared compile call.
- `crates/smelt-ui/src/run_manager.rs` — delete the local `compile_sql`; switch to the shared call.
- `crates/smelt-ui/src/build.rs` — switch the dry-run preview path (`build_run_plan`) to use `smelt_runtime::compile` with `dry_run: true` instead of bypassing compile entirely.

**Docs touched.**
- `docs/specs/architecture.md` §"Compilation pipeline" — update the prose to match the new home (this was already partially done in Phase 0 but may need refinement once the API is concrete).

**Review checklist.**
- [ ] `smelt_runtime::compile` signature does not mention `salsa::Database` or any Salsa-specific type — analysis inputs are passed in as plain data structures (per the smelt-db purity discipline). The Salsa wrapper is in the *caller*, not in `compile`.
- [ ] CLI's `commands/run.rs` no longer constructs `PrintContext` directly — that's now `smelt-runtime` internal.
- [ ] UI's `run_manager.rs` no longer constructs `PrintContext` directly.
- [ ] The compile-parity test passes byte-identical SQL output between consumers on at least: a model using `smelt.functions.<path>(...)`, a model with an ephemeral dependency, a model with explicit type casts needed, and a model with time-filter injection.
- [ ] `UnknownSmeltFn` is still surfaced as a hard error before any execution attempt, via both CLI and UI entry points.
- [ ] No regressions in `cargo test -p smelt-cli --test example_diagnostics` or `cargo test -p smelt-lsp --test example_workspaces`. (The LSP doesn't depend on `smelt-runtime` — its test should not even notice the move.)
- [ ] LSP-relevant analysis (diagnostics, type inference) is *not* touched by this phase. Verify by grepping `crates/smelt-lsp/` and `crates/smelt-db/` — no file in those crates should have been modified.

**Commit.** `feat(smelt-runtime): own SqlCompiler + emitters; CLI and UI share one compile path`

---

### Phase 3: Move selection/filter into `smelt-runtime::select`; today's UI test-filter becomes the shared function

**Goal.** Consolidate "given selectors and excludes, return the set of models that will actually execute" into one function in `smelt-runtime`. This subsumes today's UI test-filter fix (which currently lives at `crates/smelt-ui/src/run_manager.rs:195` and `crates/smelt-ui/src/build.rs:356`), the CLI's `.gen`-filter (`commands/run.rs:100-103`), the emitted-models expansion (CLI's `discover_emitted_model_files`), and the per-model target assignment.

**Pre-conditions.** Phase 2 done.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/select_parity.rs::test_test_models_excluded` — a project with both regular models and `materialization: test` models — only regular models in the selection.
- `crates/smelt-runtime/tests/select_parity.rs::test_gen_files_excluded` — a project with `*.gen.sql` files; the `.gen` files themselves are not in the selection, but the models they emit are.
- `crates/smelt-runtime/tests/select_parity.rs::test_emitted_models_included` — a `.gen.sql` file that emits two virtual models — both virtuals appear in the selection. (Today UI does not run them.)
- `crates/smelt-runtime/tests/select_parity.rs::test_selector_parsing_consistent` — `+model_a+` selects model_a and its upstream/downstream identically via both consumer entry points.
- `crates/smelt-runtime/tests/select_parity.rs::test_excludes_combined_with_selects` — `--select +model_b --exclude tag:wip` produces the same set via both consumers.
- `crates/smelt-runtime/tests/select_parity.rs::test_per_model_target_assignment` — model with `target:` metadata override gets assigned to the right backend.

**Implementation shape.**
- New module `crates/smelt-runtime/src/select.rs`:
  - `pub fn select_executable_models(graph: &DependencyGraph, config: &Config, request: &ExecuteRequest) -> Result<SelectionPlan>` where `SelectionPlan { ordered_models: Vec<String>, target_assignments: HashMap<String, String>, cross_engine_edges: Vec<(String, String)>, emitted_models: Vec<EmittedModel> }`.
  - Internally: parse selectors, apply excludes, drop tests via `model.is_test()`, drop `.gen` files, expand emitted models (calls into `smelt-db`'s existing emitted-models pipeline; does not reimplement it), assign targets per model, detect cross-engine edges via the existing `graph.find_cross_backend_edges`.
- The CLI's `commands/run.rs` selection pre-amble (lines ~100-250) collapses to one call to `smelt_runtime::select_executable_models`.
- The UI's `run_manager.rs` selection pre-amble (lines ~167-225 plus today's test-filter at line 195) collapses to the same call.
- The UI's `build.rs::build_run_plan` (the dry-run preview) also calls this function, ensuring preview parity with what would actually execute.
- Today's `.filter(|name| graph_lock.get_model(name).map(|m| !m.is_test()).unwrap_or(true))` lines in `run_manager.rs:195` and `build.rs:356` are *deleted* — the filter now lives in the shared function. Verify by `git grep '!m.is_test()'` shows it in `smelt-runtime/src/select.rs` only.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/select.rs` (new).
- `crates/smelt-runtime/src/lib.rs` — re-export.
- `crates/smelt-runtime/tests/select_parity.rs` (new).
- `crates/smelt-cli/src/commands/run.rs` — replace the selection pre-amble.
- `crates/smelt-ui/src/run_manager.rs` — delete the inline filter, replace the selection pre-amble.
- `crates/smelt-ui/src/build.rs` — replace `build_run_plan`'s selection logic.

**Docs touched.** None.

**Review checklist.**
- [ ] `select_executable_models` is the single home of the filter — `git grep 'is_test()'` shows it in `smelt-runtime` and in display-layer code (e.g. `build.rs`'s graph node type assignment) only.
- [ ] `.gen` filter is the same in CLI and UI now (UI previously had no `.gen` filter).
- [ ] Emitted-models expansion happens in both consumers (UI previously did not expand generators).
- [ ] Per-model target assignment is consistent — same model gets the same backend in both consumers.
- [ ] Selector parsing errors surface with the same error message via both consumers.
- [ ] Today's `test_*.sql` files in `examples/` still appear in the dependency graph (as `NodeType::Test`) but never in the executable selection.

**Commit.** `feat(smelt-runtime): own model selection + filter pass; CLI and UI share one selector`

---

### Phase 4: Move execute loop into `smelt-runtime::execute`; consumers shrink to surface wrappers

**Goal.** The capstone phase. Move the per-model execute loop, batch dispatch, manifest writing, and interval-store updates into `smelt-runtime::execute`. After this phase, `smelt-cli/src/commands/run.rs` and `smelt-ui/src/run_manager.rs` are thin wrappers — argument parsing + reporter setup + entry-point call. Add the end-to-end parity CI gate.

**Pre-conditions.** Phases 2 and 3 done. (Compile and selection must already flow through `smelt-runtime` so the execute loop has well-defined inputs.)

**TDD tests to write first.**
- `crates/smelt-runtime/tests/execute_parity.rs::test_full_refresh_parity` — a project with a non-incremental table model: both CLI and UI entry points produce identical row counts and identical manifest entries.
- `crates/smelt-runtime/tests/execute_parity.rs::test_incremental_parity` — incremental model with a time range: identical batch boundaries, identical row counts per batch, identical manifest entries.
- `crates/smelt-runtime/tests/execute_parity.rs::test_cumulative_aggregate_parity` — the cumulative_aggregate dispatch (recently landed) works via both consumers.
- `crates/smelt-runtime/tests/execute_parity.rs::test_function_expansion_in_execute` — model using `smelt.functions.<path>(...)` executes successfully via UI (regression test for today's silent compile divergence — this would have failed before Phase 2; this test enforces it stays fixed).
- `crates/smelt-runtime/tests/execute_parity.rs::test_test_model_not_executed` — a `materialization: test` model in the same project does not appear in either consumer's manifest. (Regression test for today's panic.)
- `crates/smelt-runtime/tests/execute_parity.rs::test_reporter_callbacks_fire` — `NoOpReporter` runs; a capture reporter records every event; the event sequence matches the spec'd order (`run_started → model_started → (batch_completed)* → model_completed → ... → run_completed`).
- `crates/smelt-runtime/tests/execute_parity.rs::test_cancellation_during_run` — cancellation midway through a multi-batch incremental run leaves a partial manifest and a `run_cancelled` event.
- `crates/smelt-runtime/tests/execute_parity.rs::test_dry_run_emits_no_sql` — `dry_run: true` produces a `RunOutcome` with the planned models but no backend calls and no manifest write.

**Implementation shape.**
- New module `crates/smelt-runtime/src/execute.rs`:
  - `pub async fn execute_project(request: ExecuteRequest, db: ..., graph: Arc<Mutex<DependencyGraph>>, config: Arc<Config>, project_dir: &Path, reporter: &dyn RunReporter, cancel: CancellationToken) -> Result<RunOutcome>`.
  - Internally: call `select_executable_models`; compute model plans (uses existing `smelt_planner::analyze_batch_safety`); create backends per needed target; run the per-model loop with reporter callbacks; write manifest + intervals via `smelt-state` (unchanged); handle cancellation; handle dry-run.
- The dry-run path (`ExecuteRequest::dry_run = true`) replaces `smelt-ui::build::build_run_plan` — there is no longer a separate preview path. The reporter receives "would have executed" events without backend calls.
- `RunReporter` gains a default-no-op `run_completed(&self, _outcome: &RunOutcome) {}` method.
- `smelt-cli/src/commands/run.rs` becomes a wrapper:
  - Parse CLI args into `ExecuteRequest`.
  - Construct a `StdoutReporter` (or whatever the CLI calls today — spinner + tracing).
  - Call `smelt_runtime::execute_project(...)`.
  - Print the final outcome.
  - Total length: ~150 lines, down from 1404.
- `smelt-ui/src/run_manager.rs` becomes a wrapper:
  - Convert HTTP `RunExecuteRequest` into `ExecuteRequest`.
  - Construct a `BroadcastReporter` that maps reporter calls to `RunProgressEvent::*` and pushes to the existing `broadcast::Sender<RunProgressEvent>`.
  - Spawn `smelt_runtime::execute_project(...)` on tokio, update the `RunManagerInner` state from a wrapper around the reporter or a select on the outcome.
  - Total length: ~200 lines, down from 709.

**Critical files (allowed to touch in this phase).**
- `crates/smelt-runtime/src/execute.rs` (new).
- `crates/smelt-runtime/src/lib.rs` — re-export `execute_project`.
- `crates/smelt-runtime/tests/execute_parity.rs` (new).
- `crates/smelt-cli/src/commands/run.rs` — gut the execute loop; keep the CLI-specific reporter and entry point.
- `crates/smelt-cli/src/reporter.rs` (new) — `StdoutReporter` impl.
- `crates/smelt-ui/src/run_manager.rs` — gut the execute loop; keep the `RunManager` state-tracking and HTTP plumbing.
- `crates/smelt-ui/src/run_reporter.rs` (new) — `BroadcastReporter` impl.
- `crates/smelt-ui/src/build.rs` — delete `build_run_plan`; the `/api/run/plan` endpoint now routes through `execute_project` with `dry_run: true`.

**Docs touched.**
- `docs/specs/architecture.md` §Surface "Compilation pipeline" — final pass to ensure the prose matches the now-fully-extracted state. (Most of this should already be true from Phase 0's anticipatory wording.)

**Review checklist.**
- [ ] `smelt-cli/src/commands/run.rs` < 250 lines.
- [ ] `smelt-ui/src/run_manager.rs` < 250 lines.
- [ ] Neither file constructs a `Backend` directly anymore — that's `smelt-runtime` internal.
- [ ] Neither file calls `inject_time_filter` or `compile_sql` or `analyze_batch_safety` directly — those are all internal to `execute_project`.
- [ ] The end-to-end parity test runs `examples/web_analytics/` through both entry points and asserts identical manifests (modulo run_id and timestamps).
- [ ] Cancellation works in both consumers (CLI: Ctrl-C; UI: cancel endpoint).
- [ ] `RunReporter` events fire in the same order via both consumers — the captured event sequence in the parity test is identical.
- [ ] LSP is untouched. Verify by `git diff --stat` over `crates/smelt-lsp/` showing zero lines changed in this phase.
- [ ] The `/api/run/plan` UI endpoint still works (it now routes through `dry_run: true`).

**Commit.** `feat(smelt-runtime): own execute loop; CLI and UI become thin surface wrappers`

---

### Phase 5: Surface tightening; delete consumer-side duplicates; final crate-graph audit

**Goal.** Lock in the architectural rule by making it structurally hard to bypass. Tighten `smelt-runtime`'s public surface — internals become `pub(crate)`. Delete consumer-side helpers that the transitional re-exports kept alive. Audit the crate dependency graph to confirm no consumer reaches into `smelt-runtime` internals or duplicates logic.

**Pre-conditions.** Phases 2–4 done.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/surface_audit.rs::test_no_compiler_internals_exposed` — a compile-time assertion (`fn _assert<T: ?Sized>() {}`) over the public surface confirms `SqlCompiler` / emitter factories / `PrintContext` constructors are not reachable from outside `smelt-runtime`.
- `crates/smelt-runtime/tests/surface_audit.rs::test_run_reporter_is_object_safe` — `Box<dyn RunReporter>` compiles.
- Standing CI gates green.

**Implementation shape.**
- In `crates/smelt-runtime/src/`: change every helper that the public API no longer needs to `pub(crate)`. Public surface should be roughly: `ExecuteRequest`, `RunOutcome`, `RunReporter`, `NoOpReporter`, `execute_project`, `compile`, `select_executable_models`, `inject_time_filter` (if still externally useful), `build_fn_body_map*`, and the diagnostic-code constants used by the gate.
- Delete the transitional re-exports in `smelt-cli/src/lib.rs` (added in Phase 1) that kept `inject_time_filter` etc. visible to other parts of `smelt-cli`. Callers should now import from `smelt_runtime` directly.
- Delete `smelt-cli/src/compiler.rs` if it's been hollowed out to a thin shim. The CLI's `commands/run.rs` imports from `smelt-runtime` instead.
- Delete `smelt-ui/src/build.rs::build_run_plan` (already routed through `execute_project` in Phase 4); delete the helper functions it pulled in.
- Run `cargo +nightly udeps` (or `cargo-machete`) to detect now-unused dependencies in `smelt-cli` and `smelt-ui` Cargo.tomls; remove them.

**Critical files (allowed to touch in this phase).**
- All files in `crates/smelt-runtime/src/` — visibility audit.
- `crates/smelt-cli/src/lib.rs` — drop transitional re-exports.
- `crates/smelt-cli/src/compiler.rs` — delete if empty.
- `crates/smelt-cli/Cargo.toml` — prune unused deps.
- `crates/smelt-ui/Cargo.toml` — prune unused deps.
- `crates/smelt-ui/src/build.rs` — final cleanup.

**Docs touched.**
- `docs/specs/architecture.md` — final pass over the "Crate responsibilities" table to ensure `smelt-runtime`'s row accurately names its public surface.

**Review checklist.**
- [ ] `cargo doc -p smelt-runtime` shows only the intended public surface — no `SqlCompiler`, no emitter factories, no `PrintContext` constructors.
- [ ] Consumer crates cannot construct a `CompiledModel` half-way (e.g. with type casts but no fn expansion) — the public API forces "all or nothing."
- [ ] `cargo machete` (or equivalent) reports no unused dependencies in `smelt-cli` or `smelt-ui`.
- [ ] All Phase 2/3/4 parity tests still green.
- [ ] LSP unaffected; `smelt-lsp`'s Cargo.toml does not include `smelt-runtime` as a dependency.
- [ ] Manual crate-graph inspection: `cargo tree -p smelt-runtime` shows `smelt-runtime → {smelt-db, smelt-core, smelt-parser, smelt-planner, smelt-backend, smelt-state, smelt-dialect}` and nothing pointing back at it from the analysis layer.

**Commit.** `chore(smelt-runtime): tighten public surface; delete consumer-side duplicates`

---

## Post-merge follow-ups (out of plan)

- **`smelt-language-service` extraction.** Triggered by UI-editor feature work, not by this plan. The architecture spec's Known Divergences entry tracks the slot. The plan's discipline (no analysis logic in `smelt-runtime`, LSP untouched) preserves the option.
- **LSP dry-compile.** Now that `smelt_runtime::compile` is a pure function returning `CompiledModel`, the LSP could call it to surface type-cast diagnostics in the editor. Separable design question; out of scope here.
- **Spark in the UI.** With `smelt-runtime` owning execution, the UI's Spark gap closes for free (or at least: shrinks to the same gap the CLI has).
- **Eliminate `smelt-cli`'s lib portion.** With `smelt-runtime` doing the heavy lifting, `smelt-cli` can become a pure binary crate (no `lib.rs`). Defer until a CLI tooling consumer makes the lib portion necessary again.

## References

- Predecessor research: `docs/research/20260523-lsp-cli-ui-divergence.md` — concrete divergence catalogue with file:line citations; design rationale for the layered architecture; tradeoffs around the language-service slot.
- Today's bug fix: `crates/smelt-ui/src/run_manager.rs:195`, `crates/smelt-ui/src/build.rs:356` — `is_test()` filter that this plan generalises into `smelt_runtime::select_executable_models`.
- Predecessor parity rules: `docs/specs/architecture.md` §"Workspace loading parity rule (CLI ↔ LSP)" and §"Project isolation rule" — same shape, same enforcement story.
- Pure-function discipline: `docs/specs/architecture.md` §"Salsa purity rule (analysis)" — preserved by this plan; analysis logic stays in `smelt-db`.
