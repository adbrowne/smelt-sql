# Plan: CLI execute-loop migration + runtime surface lockdown

**Date**: 2026-05-24
**Spec**: [`docs/specs/architecture.md`](../specs/architecture.md)
**Spec diff**: extend the existing §"Run pipeline parity rule (CLI ↔ UI)" with the structural-enforcement clause (internals `pub(crate)`); update the §"Crate responsibilities" table to drop CLI's `LogicalGraph` / `PhysicalGraph` rows once they are inlined; update CLAUDE.md's "Run Pipeline Parity Rule" subsection in lockstep.
**Tracking branch**: TBD when work begins (suggested: `feat/cli-runtime-migration`).
**Docs**: code-only for the bulk; spec touch in Phase 7 only.
**Predecessor plan**: [`docs/plans/20260523-smelt-runtime-extraction.md`](20260523-smelt-runtime-extraction.md) (phases 0–4a done, 4b–5 partial).
**Predecessor research**: [`docs/research/20260523-lsp-cli-ui-divergence.md`](../research/20260523-lsp-cli-ui-divergence.md).
**Motivating example**: with the runtime in place but the CLI still owning the bulk of the execute loop (~1400 lines in `crates/smelt-cli/src/commands/run.rs` plus ~2400 lines across `temporal.rs`, `logical_graph.rs`, `physical_graph.rs`), every new lifecycle feature still needs two implementations and the standing CI gate the parity rule promises (`cargo test -p smelt-runtime --test execute_parity`) cannot exist yet.

---

## Execution prompt (for a fresh Claude session)

You are executing this plan from the start of a new session.

**Before touching any code:**

1. Read this entire plan file. Then read `docs/specs/architecture.md` (the correctness oracle) and the predecessor plan at `docs/plans/20260523-smelt-runtime-extraction.md` (so you know what is *already* in `smelt-runtime`).
2. Read `docs/research/20260523-lsp-cli-ui-divergence.md` for the load-bearing rationale — do not re-open settled decisions.
3. Confirm you are on the tracking branch (a fresh feature branch off whatever the project's main branch is at that point — *not* `worktree-web_analytics`, which has already shipped phases 0–5 partial).
4. Find the next `pending` phase in Progress tracking.

**For each phase:** implementer subagent → reviewer subagent → iterate → record + commit + push.

**Conventions every phase:**
- Real-fixture tests in `examples/` for any behaviour change. `examples/web_analytics/` (incremental + cumulative + functions), `examples/ephemeral_demo/` (ephemerals), and `examples/timeseries/` (bounded windows) are the load-bearing fixtures.
- Red-green TDD: failing test before any implementation.
- Atomic per-phase commits using the phase's `Commit.` line verbatim.
- Never skip hooks, never `--no-verify`, never force-push.
- Honour `CLAUDE.md` invariants and the existing parity rules. **None of these are weakened by this plan.** The Workspace Loading Parity Rule, Project Isolation Rule, Pure Function Rule, and the (partially-enforced) Run Pipeline Parity Rule all continue to apply.
- **Timeless-oracle rule.** Phase vocabulary lives in *this plan file only*. Edits to `docs/specs/architecture.md` and `CLAUDE.md` describe the architecture as if it has always existed.
- **No CLI feature regressions.** The CLI today refuses execution on planner-detected incremental safety failures (unless `--allow-downgrade`), refuses on undefinable temporal bounds, supports `--show-plan` / `--verbose`, and prints rich progress to stdout. Every one of these must continue to work after migration — verified by the existing CLI test suite, which must stay green at every push.

---

## Context

The predecessor plan extracted compile, selection, execute_project, and cumulative dispatch into `smelt-runtime`. The UI consumes the shared runtime end-to-end. The CLI, however, still runs its own ~700-line execute loop because:

1. **Per-source temporal bounds (`compute_incremental_windows`)** — the CLI's incremental loop iterates batch windows computed from planner-derived per-source bounds (`derive_model_source_bounds(model_info, &opt_graph)`). The runtime's current `execute_project` uses a simpler `analyze_batch_safety` path. Replacing the CLI's loop with the runtime's loop would lose per-source bound adjustment — a real correctness feature for incremental models that depend on multiple sources with different temporal shapes.
2. **Planner safety check + temporal bound derivation** — `Planner::new().plan(&opt_graph)` runs an incremental safety classifier; failures are refused at planning time unless `--allow-downgrade`. The runtime doesn't currently run this check, so a CLI migration without lifting it would lose the refusal behaviour.
3. **`PhysicalGraph` / `LogicalGraph`** — the CLI's execute loop iterates a `PhysicalGraph` whose nodes carry baked-in target / materialization / plan-step / ephemeral-resolver state. The runtime's `execute_project` doesn't use this concept — it walks `DependencyGraph` and resolves per-model state at execute time. The two structures are not interchangeable without refactoring.
4. **Schema-evolution checks** — the CLI's loop runs a per-model schema-evolution gate (`force_full_refresh` logic based on `SchemaEvolutionStrategy` metadata and `--allow-column-removal` / `--allow-full-refresh` flags). The runtime doesn't run this; migrating the CLI loses the gate unless the runtime gains it.
5. **CLI-specific UX** — `--show-plan`, `-v / --verbose` (SQL output per model), `--show-results` (result preview), CLI stdout progress format. These are reporter-shape concerns but need a clean hookup.

This plan closes those gaps in seven phases. Each phase is independently shippable; the order is chosen so that each phase's tests can rely on the previous phase's outputs without speculative coupling. Phases 1–5 unify the execute path (CLI gains `execute_project`); phases 6–7 lock the surface so future drift is structurally impossible.

A note on `smelt backbuild`: this plan does **not** migrate `commands/backbuild.rs` to `execute_project`. Backbuild is a separate command with its own backfill loop and intentionally calls `compute_backbuild_plans` + iterates windows directly. Its migration is a separable follow-up; this plan keeps it on its current path.

## Scope

### In scope

- **`compute_incremental_windows`** and the per-source-bound machinery move from `smelt-cli/src/temporal.rs` into `smelt-runtime`. The runtime's batch planner switches from `analyze_batch_safety`-only to bound-aware windowing.
- **Planner safety check + temporal bound derivation** move into `smelt-runtime::execute_project` as a gated pre-execute pass. `ExecuteRequest` gains `enforce_safety: bool` (default `true`); CLI sets `enforce_safety = !args.allow_downgrade`. UI defaults to `true` so it gets the same refusal protection.
- **`LogicalGraph` / `PhysicalGraph`** are either moved into `smelt-runtime` or eliminated. Recommendation: eliminate. `execute_project` already computes ephemeral resolvers and target assignments from `DependencyGraph` + `Config`; the remaining `PhysicalGraph` responsibilities (planner-applied strategy overrides, `--show-plan` summaries) are handled by a smaller `PlanSummary` value the runtime returns alongside `RunOutcome`.
- **Schema-evolution checks** (`force_full_refresh` gate, `--allow-column-removal`, `--allow-full-refresh`) lift into the runtime. `ExecuteRequest` gains the two `allow_*` flags.
- **`StdoutReporter`** in `smelt-cli` — implements `RunReporter` and prints the CLI's current progress format. The CLI's execute loop reduces to: build `ExecuteRequest` from args → construct `StdoutReporter` → call `execute_project` → print summary.
- **`-v / --verbose` and `--show-plan`** — handled via a new `RunReporter::model_compiled(name, sql)` callback that the runtime fires after each compile and before execute. The reporter chooses whether to print.
- **End-to-end parity CI gate** — `cargo test -p smelt-runtime --test execute_parity` runs the same project (`examples/web_analytics/`) through both CLI and UI entry points, asserts identical manifests and identical row counts per model.
- **`pub(crate)` lockdown** of `SqlCompiler` constructors, `PrintContext` builders, and emitter factories in `smelt-runtime`. After lockdown, consumers can only obtain `CompiledModel` via `execute_project` or the registry's `compile_with_*` methods — half-compiled construction is structurally impossible.
- **Deletion of `smelt-cli`'s shim modules** (`src/compiler.rs`, `src/cumulative.rs`, `src/transformer.rs`) and the corresponding `lib.rs` re-exports. External callers (tests, downstream) update to `smelt_runtime::*`.

### Explicitly deferred

- **`smelt backbuild` migration.** Separate command, separate loop. Defer to a follow-up plan.
- **`smelt test` migration.** `commands/test.rs` already uses `smelt_runtime::build_fn_body_map_from_model_files`; the test runner has its own non-execute-loop machinery that doesn't fit `execute_project`. Leave alone.
- **Dry-run rich output beyond `--show-plan`.** Today's `--dry-run` prints compiled SQL per model; with `model_compiled` callbacks this becomes a reporter behaviour. No new dry-run features.
- **Spark backend in the UI.** Today UI's `BackendFactory` bails on Spark; this plan doesn't fix that.
- **`smelt-language-service` extraction** for in-browser editing. Documented in `architecture.md` → Known Divergences ("Language-service slot is empty"); awaits the UI-editor feature work, not this plan.

## Progress tracking

| Phase | Status   | Commit | Date |
|-------|----------|--------|------|
| 1 — Move `compute_incremental_windows` + per-source bound helpers into `smelt-runtime`. `execute_project`'s batch planner switches to bound-aware windowing. | pending |  |  |
| 2 — Move planner safety check + temporal bound derivation into `smelt-runtime`. `ExecuteRequest` gains `enforce_safety` + schema-evolution flags. | pending |  |  |
| 3 — Eliminate `PhysicalGraph` / `LogicalGraph`. Runtime returns `PlanSummary` for `--show-plan`. CLI consumes the smaller surface. | pending |  |  |
| 4 — Migrate `smelt-cli/src/commands/run.rs` to `execute_project`. Add `StdoutReporter`. Add `RunReporter::model_compiled` callback. | pending |  |  |
| 5 — End-to-end CLI ↔ UI parity CI gate: `cargo test -p smelt-runtime --test execute_parity` runs identical fixtures through both entry points. | pending |  |  |
| 6 — Surface lockdown: `pub(crate)` on `SqlCompiler` constructors, `PrintContext` builders, emitter factories. Half-compile construction becomes a type error. | pending |  |  |
| 7 — Delete `smelt-cli`'s shim modules + lib.rs re-exports. Tests and external callers move to `smelt_runtime::*`. Spec update lands. | pending |  |  |

---

### Phase 1: `compute_incremental_windows` + per-source bounds into runtime

**Goal.** Move `crates/smelt-cli/src/temporal.rs` (~391 lines: `compute_incremental_windows`, `validate_run_window_alignment`, `IncrementalWindows`, `EffectiveWindow`) and the `smelt_planner::derive_model_source_bounds` orchestration into `smelt-runtime`. Update `execute_project`'s batch planner to use bound-aware windowing instead of the simpler `analyze_batch_safety` path. Result: incremental models with multi-source bounds (e.g. `examples/web_analytics/`) batch identically via CLI and UI.

**Pre-conditions.** None — entry point.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/windowing_parity.rs::test_multi_source_bound_aware_windows` — a fixture model with two timeseries sources of different `before_secs`/`after_secs` produces the same `(partition_start, partition_end, filter_start, filter_end)` shapes as today's CLI loop. Compare against captured-output reference.
- `crates/smelt-runtime/tests/windowing_parity.rs::test_lookback_widens_filter_window` — a source with `Bounded { context_days: 3 }` widens `filter_start` by 3 days while `partition_start` stays at the partition boundary.
- `crates/smelt-runtime/tests/windowing_parity.rs::test_per_partition_override` — `request.per_partition = true` produces one batch per granularity period, ignoring `BatchSafety::FullyBatchSafe`.
- `crates/smelt-runtime/tests/windowing_parity.rs::test_batch_size_days_override` — `request.batch_size_days = Some(2)` produces 2-day batches.
- `crates/smelt-runtime/tests/windowing_parity.rs::test_validate_run_window_alignment_misalignment` — a run window that doesn't align to granularity boundaries surfaces the same error message as today's CLI helper.
- Existing CLI suite (`example_diagnostics`, `cumulative_equivalence`) stays green.

**Implementation shape.**
- New module `crates/smelt-runtime/src/windowing.rs`.
  - Move `IncrementalWindows`, `EffectiveWindow`, `compute_incremental_windows`, `validate_run_window_alignment`.
  - `compute_incremental_windows` takes `(spec, full_range, model_info, dep_timeseries, batch_size_days, per_partition) -> IncrementalWindows`.
  - The `dep_timeseries` map is built by the caller from `DependencyGraph`'s nodes (each dep's `metadata.timeseries`).
- `smelt_runtime::execute_project` replaces its inline `analyze_batch_safety`-based batch construction with `compute_incremental_windows`. The new shape is bound-aware.
- `smelt-cli/src/temporal.rs` collapses to a re-export shim: `pub use smelt_runtime::windowing::*;`.

**Critical files.**
- `crates/smelt-runtime/src/windowing.rs` (new).
- `crates/smelt-runtime/src/lib.rs` — re-export `compute_incremental_windows`, `IncrementalWindows`, etc.
- `crates/smelt-runtime/src/execute.rs` — swap batch planner.
- `crates/smelt-runtime/tests/windowing_parity.rs` (new).
- `crates/smelt-cli/src/temporal.rs` — shim.

**Docs touched.** None this phase. Spec update is Phase 7.

**Review checklist.**
- [ ] `windowing_parity` tests all green.
- [ ] `cumulative_equivalence` integration test still green (it depends on per-source bound handling for its cumulative loop).
- [ ] `example_diagnostics` and `example_workspaces` gates green.
- [ ] No `dep_timeseries` flat lookup leaks across projects (Project Isolation Rule).
- [ ] `analyze_batch_safety`'s `BatchSafety::PerPartitionOnly` semantics preserved.

**Commit.** `feat(smelt-runtime): bound-aware windowing in execute_project`

---

### Phase 2: Planner safety check + temporal bound derivation into runtime

**Goal.** Move the planner safety check (`Planner::new().plan(&opt_graph)` refusing incremental safety failures) and temporal bound derivation (`derive_model_source_bounds` refusing undefinable bounds) into `smelt-runtime`. Add `ExecuteRequest::enforce_safety: bool` (default `true`) and `ExecuteRequest::allow_column_removal` / `allow_full_refresh` for schema evolution. Result: the UI gets safety protection for free; the CLI sets `enforce_safety = !args.allow_downgrade`.

**Pre-conditions.** Phase 1 done — `compute_incremental_windows` is the windowing primitive Phase 2's bound-derivation step writes into.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/safety_check_parity.rs::test_unsafe_incremental_refused_by_default` — a model with a planner-detected incremental safety failure is refused; the error message matches the CLI's current refusal.
- `crates/smelt-runtime/tests/safety_check_parity.rs::test_unsafe_incremental_allowed_with_enforce_safety_false` — same model with `enforce_safety = false` runs (falls back to full-table refresh, with a warning).
- `crates/smelt-runtime/tests/safety_check_parity.rs::test_undefinable_bound_refused_by_default` — a model with a bare `LAG` over a timeseries source (no `RANGE BETWEEN INTERVAL`) is refused; matches CLI behaviour.
- `crates/smelt-runtime/tests/safety_check_parity.rs::test_schema_evolution_blocks_column_removal_by_default` — a model whose deployed schema has a column removed is refused unless `allow_column_removal = true`.
- Existing CLI suite stays green — verify by running `cargo test -p smelt-cli` after Phase 2 lands.

**Implementation shape.**
- `ExecuteRequest` gains: `enforce_safety: bool` (default `true` via `#[serde(default = "true_")]`), `allow_column_removal: bool`, `allow_full_refresh: bool`.
- `execute_project` runs the safety check as a pre-execute pass when `enforce_safety` is true. The orchestration moves out of `commands/run.rs` (lines ~455-532) into `smelt-runtime`.
- The check builds the `ModelGraph` from the runtime's `DependencyGraph` + parsed frontmatter, then calls `smelt_planner::Planner::new().plan(...)` and `derive_model_source_bounds`.
- Schema evolution check moves into `execute_project`'s per-model loop. The `force_full_refresh` logic + `--allow-*` flag handling lives behind a single `should_force_full_refresh(model, deployed_schema, allow_column_removal, allow_full_refresh) -> bool` helper.

**Critical files.**
- `crates/smelt-runtime/src/types.rs` — add fields to `ExecuteRequest`.
- `crates/smelt-runtime/src/execute.rs` — wire the safety pass + schema-evolution gate.
- `crates/smelt-runtime/src/safety.rs` (new) — orchestration helpers (`build_planner_input`, `should_force_full_refresh`).
- `crates/smelt-runtime/tests/safety_check_parity.rs` (new).

**Docs touched.** None this phase.

**Review checklist.**
- [ ] `enforce_safety = true` default — the UI gets refusal protection.
- [ ] `enforce_safety = false` matches CLI's `--allow-downgrade` warning-log behaviour exactly.
- [ ] Schema-evolution flag semantics match the CLI's existing `--allow-column-removal` / `--allow-full-refresh` behaviour.
- [ ] Refusal error messages match the CLI's existing strings (use the same `format!` templates).
- [ ] Bound derivation is keyed by `ProjectInput` (Project Isolation Rule).

**Commit.** `feat(smelt-runtime): planner safety check + schema-evolution gate`

---

### Phase 3: Eliminate `PhysicalGraph` / `LogicalGraph`

**Goal.** Remove the CLI-specific `LogicalGraph` (884 lines) and `PhysicalGraph` (1184 lines). `execute_project` already does what they do (selection, target assignment, ephemeral resolvers); the remaining concerns (planner-applied strategy overrides, `--show-plan` summaries) move into a small `PlanSummary` value the runtime returns alongside `RunOutcome`.

**Pre-conditions.** Phases 1 and 2 done.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/plan_summary.rs::test_plan_summary_lists_strategies` — `execute_project` with `dry_run = true` returns a `PlanSummary` whose entries name each model + resolved strategy (full refresh / incremental / cumulative).
- `crates/smelt-runtime/tests/plan_summary.rs::test_planner_override_applied` — a model with a planner rule that turns it incremental shows up as `Incremental` in the `PlanSummary` even if its frontmatter says `Table`.
- `crates/smelt-runtime/tests/plan_summary.rs::test_dry_run_emits_no_backend_calls` — `dry_run = true` returns `PlanSummary` but does not invoke `BackendFactory::create`.
- `crates/smelt-cli` tests that depend on `LogicalGraph` / `PhysicalGraph` (`tests/logical_graph_test.rs`, etc.) are either deleted or rewritten against the new surface.

**Implementation shape.**
- New `PlanSummary { models: Vec<ModelPlanRecord> }` in `smelt-runtime::types`.
- `execute_project` produces `PlanSummary` during its model-plan construction; with `dry_run = true` it returns the summary and skips execution.
- `crates/smelt-cli/src/logical_graph.rs` and `crates/smelt-cli/src/physical_graph.rs` are deleted. Their callers (`commands/run.rs`, `commands/explain.rs`, `commands/diff.rs`, `commands/backbuild.rs`) update.
- The CLI's `commands/explain.rs` switches from constructing a `PhysicalGraph` to calling `execute_project(request_with_dry_run, ...)` and rendering the `PlanSummary`.

**Critical files.**
- `crates/smelt-runtime/src/types.rs` — add `PlanSummary`, `ModelPlanRecord`.
- `crates/smelt-runtime/src/execute.rs` — dry-run path produces the summary.
- `crates/smelt-cli/src/{logical_graph,physical_graph}.rs` — deleted.
- `crates/smelt-cli/src/lib.rs` — drop the re-exports.
- `crates/smelt-cli/src/commands/{run,explain,diff,backbuild}.rs` — switch to the new surface.

**Docs touched.** `docs/specs/architecture.md` — drop `LogicalGraph` / `PhysicalGraph` mentions if any (lazily; Phase 7 does the proper spec update).

**Review checklist.**
- [ ] `--show-plan` output unchanged byte-for-byte (the CLI's renderer of `PlanSummary` produces the same text).
- [ ] `commands/explain.rs` works on every example workspace.
- [ ] No `PhysicalGraph` / `LogicalGraph` references remain in `crates/smelt-cli/src/`.
- [ ] `cargo test -p smelt-cli` stays green.

**Commit.** `refactor(smelt-runtime): subsume LogicalGraph and PhysicalGraph; PlanSummary surface`

---

### Phase 4: CLI `commands/run.rs` migrates to `execute_project`

**Goal.** Replace `crates/smelt-cli/src/commands/run.rs`'s execute loop (lines ~601–1282) with a call to `smelt_runtime::execute_project`. Add `StdoutReporter` (implements `RunReporter`, prints CLI's existing progress format). Add `RunReporter::model_compiled(name, sql)` callback for `-v` / `--verbose`. Final `commands/run.rs` is < 250 lines.

**Pre-conditions.** Phases 1–3 done.

**TDD tests to write first.**
- `crates/smelt-cli/tests/run_command_end_to_end.rs::test_full_refresh_run` — `smelt run` against a small fixture produces the expected manifest and the expected stdout output.
- `crates/smelt-cli/tests/run_command_end_to_end.rs::test_verbose_prints_compiled_sql` — `smelt run -v` prints the compiled SQL via the `model_compiled` callback.
- `crates/smelt-cli/tests/run_command_end_to_end.rs::test_allow_downgrade_warns_and_runs` — `smelt run --allow-downgrade` against an unsafe incremental model warns and runs.
- `crates/smelt-cli/tests/run_command_end_to_end.rs::test_show_plan_dry_run` — `smelt run --dry-run --show-plan` prints the plan summary.

**Implementation shape.**
- New `crates/smelt-cli/src/reporter.rs` with `StdoutReporter`:
  - `model_started` → `info!("Running model: {name} ({idx}/{total})")` etc.
  - `model_compiled(name, sql)` → if `verbose`, `println!("-- {name}\n{sql}");`.
  - `model_completed` → `info!("  {name} done ({rows} rows, {duration:?})")`.
  - `run_completed` → final summary.
  - Holds a `verbose: bool` and a `show_results: bool`.
- `RunReporter::model_compiled` added to the trait (default no-op). UI's `BroadcastReporter` ignores it (UI doesn't surface compiled SQL today).
- `commands/run.rs` body:
  1. Parse args → build `ExecuteRequest`.
  2. Build `DependencyGraph` from discovered models.
  3. Open Salsa DB.
  4. Construct `StdoutReporter` from `args.verbose` / `args.show_results`.
  5. Construct `CliBackendFactory` (DuckDB + Spark).
  6. Call `execute_project(...)`.
  7. Print the final summary from `RunOutcome`.
- `CliBackendFactory` lives in `crates/smelt-cli/src/backend_factory.rs`. Handles DuckDB and Spark target creation (today's `BackendRegistry` logic).

**Critical files.**
- `crates/smelt-cli/src/reporter.rs` (new).
- `crates/smelt-cli/src/backend_factory.rs` (new).
- `crates/smelt-cli/src/commands/run.rs` — gut the execute loop, keep the CLI-specific arg parsing.
- `crates/smelt-runtime/src/reporter.rs` — add `model_compiled` default-noop method.
- `crates/smelt-cli/tests/run_command_end_to_end.rs` (new).

**Docs touched.** None this phase.

**Review checklist.**
- [ ] `commands/run.rs` < 250 lines.
- [ ] Stdout output is byte-identical to today's CLI run on a baseline fixture (test captures stdout and diffs against a checked-in golden file).
- [ ] All CLI flags (`--target`, `--dry-run`, `--show-plan`, `--verbose`, `--show-results`, `--per-partition`, `--batch-size`, `--allow-downgrade`, `--allow-column-removal`, `--allow-full-refresh`, `--select`, `--exclude`, `--event-time-start`, `--event-time-end`) behave identically.
- [ ] `commands/run.rs` no longer constructs `SqlCompiler` or `Backend` directly — both flow through the runtime.
- [ ] LSP untouched (`git diff --stat crates/smelt-lsp/` shows 0 lines).

**Commit.** `feat(smelt-cli): commands/run.rs is a thin wrapper over execute_project`

---

### Phase 5: End-to-end CLI ↔ UI parity CI gate

**Goal.** Add the standing CI gate the parity rule promises: `cargo test -p smelt-runtime --test execute_parity` runs `examples/web_analytics/` through both CLI and UI entry points and asserts identical manifest contents (modulo run_id and timestamps) and identical row counts per model. The fixture set covers the Mode-B classes: test models (filter), generators (expansion), `smelt.fn.*` calls (compile), incremental models (batch dispatch), cumulative aggregates (rule dispatch), ephemeral CTEs (inlining).

**Pre-conditions.** Phase 4 done — both consumers must already call `execute_project`.

**TDD tests to write first.** This phase *is* the test.

**Implementation shape.**
- `crates/smelt-runtime/tests/execute_parity.rs`:
  - `test_cli_ui_manifest_parity_web_analytics` — boot a `DuckDbBackend` against a temp `.duckdb` file. Run the project through `execute_project` once with a `StdoutReporter`-equivalent reporter, capture the `RunOutcome`. Run again with a captured-events reporter mimicking the UI's reporter. Assert the two outcomes match on `models` keys, per-model `row_count`, `strategy`, and `partitions_updated`.
  - Smaller targeted variants: `_with_cumulative`, `_with_generators`, `_with_test_models`, `_with_ephemerals`, `_with_smelt_fn_calls`.
- The test is wired up in `crates/smelt-runtime/Cargo.toml` `[[test]]` section.
- CLAUDE.md gains a one-line mention under the "Run Pipeline Parity Rule" subsection: "Standing CI gate: `cargo test -p smelt-runtime --test execute_parity`."

**Critical files.**
- `crates/smelt-runtime/tests/execute_parity.rs` (new).
- `CLAUDE.md` — gate name update under the existing parity rule subsection.

**Docs touched.** `CLAUDE.md` — gate name only. Spec update is Phase 7.

**Review checklist.**
- [ ] The test runs all six Mode-B fixtures.
- [ ] The test uses a real DuckDB backend (not a mock) so the parity assertion exercises the actual compile + execute path.
- [ ] Test runs in CI under `cargo test --workspace`; no special flags required.
- [ ] Failure messages clearly identify which fixture and which assertion diverged.

**Commit.** `test(smelt-runtime): end-to-end CLI ↔ UI parity CI gate`

---

### Phase 6: Surface lockdown — `pub(crate)` on compile internals

**Goal.** Make the structural enforcement clause of the Run Pipeline Parity Rule real. After this phase, a consumer cannot construct a `CompiledModel` half-way (e.g. with type casts but no fn expansion) — the type system forbids it.

**Pre-conditions.** Phase 4 done — the CLI must already use `execute_project` so it no longer constructs `SqlCompiler` directly.

**TDD tests to write first.**
- `crates/smelt-runtime/tests/surface_audit.rs::test_no_compiler_internals_exposed` — compile-time `fn _assert<T: ?Sized>()` over the public surface confirms `SqlCompiler::new` (or whichever constructor remains pub) returns a type whose only methods are the high-level `compile_with_*` entry points; `PrintContext` is not in scope from outside.
- `crates/smelt-runtime/tests/surface_audit.rs::test_run_reporter_is_object_safe` — `Box<dyn RunReporter>` compiles.

**Implementation shape.**
- In `crates/smelt-runtime/src/compile.rs`:
  - `SqlCompiler::new` becomes `pub(crate)` — only `CompilerRegistry::new` (the public entry) can construct one.
  - `SqlCompiler::{set_cross_engine_refs, set_upstream_schemas, set_function_bodies, set_function_bodies_arc}` become `pub(crate)`.
  - `PrintContext` constructors become `pub(crate)`.
  - Emitter factories (`make_path_ref_resolver`, `build_emitters`, etc.) become `pub(crate)`.
  - `compile_with_sql` (the no-ephemerals variant) becomes `pub(crate)` — public callers must use `compile_with_sql_and_ephemerals` so they cannot accidentally bypass ephemeral inlining.
- `bind_named_args`, `substitute_params_with_named`, `resolve_refs_in_sql`, `prepend_ephemeral_ctes` — audit: only `pub` if a public caller actually needs them. `pub(crate)` otherwise.

**Critical files.**
- All files in `crates/smelt-runtime/src/`.
- `crates/smelt-runtime/tests/surface_audit.rs` (new).

**Docs touched.** None this phase.

**Review checklist.**
- [ ] `cargo doc -p smelt-runtime` shows only the intended public surface — no `SqlCompiler::new`, no `PrintContext` constructors, no emitter factories.
- [ ] All Phase 2/3/4 parity tests still green.
- [ ] LSP untouched.
- [ ] The CLI's `commands/run.rs` and the UI's `RunManager` both compile without any direct `SqlCompiler` / `PrintContext` references.

**Commit.** `chore(smelt-runtime): pub(crate) compile internals to lock in structural enforcement`

---

### Phase 7: Delete shims; spec lands the structural-enforcement clause

**Goal.** Clean up. Delete `crates/smelt-cli/src/{compiler,cumulative,transformer}.rs` (now-empty re-export shims) and the corresponding `lib.rs` re-exports. External callers (tests, downstream) update to `smelt_runtime::*`. The spec gains the structural-enforcement clause that's now real.

**Pre-conditions.** Phases 1–6 done. Phase 6 is the structural enforcement; this phase records it.

**TDD tests to write first.** Standing CI gates green is the test.

**Implementation shape.**
- Delete `crates/smelt-cli/src/compiler.rs`, `crates/smelt-cli/src/cumulative.rs`, `crates/smelt-cli/src/transformer.rs`.
- `crates/smelt-cli/src/lib.rs` — drop the `pub mod compiler;` / `pub mod cumulative;` / `pub mod transformer;` declarations and the `pub use compiler::*` / `pub use cumulative::*` / `pub use transformer::*` re-exports.
- Search for any remaining `smelt_cli::SqlCompiler` / `smelt_cli::TimeRange` / etc. callers (tests, downstream) and update to `smelt_runtime::*`. Likely affected: existing CLI tests, possibly user-facing docs.
- `docs/specs/architecture.md` updates:
  - §"Run pipeline parity rule (CLI ↔ UI)" — add the clause: "Internals of `smelt-runtime` are `pub(crate)` where they would let a consumer reach into the compile pipeline and construct partial state. Consumers can only obtain a `CompiledModel` via `execute_project` or `CompilerRegistry::get(...).compile_with_sql_and_ephemerals(...)`."
  - §"Crate responsibilities" — drop `LogicalGraph` / `PhysicalGraph` mentions if any remain.
  - Standing CI gate line updated to name `execute_parity` test.
- `CLAUDE.md` mirrors the spec.

**Critical files.**
- `crates/smelt-cli/src/{compiler,cumulative,transformer}.rs` — deleted.
- `crates/smelt-cli/src/lib.rs` — drop re-exports.
- `crates/smelt-cli/tests/*` — update imports if any still use `smelt_cli::*` for the moved types.
- `docs/specs/architecture.md`.
- `CLAUDE.md`.

**Docs touched.** `docs/specs/architecture.md`, `CLAUDE.md`. Timeless edits — reader cannot tell this is a Phase 7 change.

**Review checklist.**
- [ ] `git grep 'smelt_cli::SqlCompiler\|smelt_cli::CompilerRegistry\|smelt_cli::FnBodyMap\|smelt_cli::TimeRange\|smelt_cli::inject_time_filter\|smelt_cli::build_fn_body_map\|smelt_cli::CompiledModel\|smelt_cli::EphemeralResolver\|smelt_cli::UpstreamSchemas'` returns no hits in `crates/`.
- [ ] `cargo test --workspace` green.
- [ ] `cargo doc -p smelt-cli` shows no `compiler` / `cumulative` / `transformer` modules.
- [ ] Spec edit is timeless — no "this plan" or phase vocabulary in `architecture.md` or `CLAUDE.md`.

**Commit.** `chore(smelt-cli,spec): delete shims; land structural-enforcement clause`

---

## Tradeoffs and risks

- **Touching the CLI's well-tested execute loop** is the highest-risk part of this plan. Mitigations: (a) every phase ships with parity tests; (b) the existing CLI integration tests (`cumulative_equivalence`, `example_diagnostics`) act as standing CI; (c) Phases 1–3 prep the runtime before Phase 4 touches `commands/run.rs`, so when the migration lands the runtime is already feature-complete for the CLI's needs.
- **`compute_incremental_windows` is intricate** — it deals with bounded vs unbounded sources, lookback windows, per-partition overrides. Moving it to runtime without subtle drift requires the Phase 1 parity tests to be exhaustive. Capture reference outputs from the *current* CLI implementation before touching anything.
- **`PlanSummary` shape question** — Phase 3 collapses `LogicalGraph` / `PhysicalGraph` into a smaller value. There's a tension between "rich enough to power `--show-plan`" and "minimal enough not to recreate the same type bloat under a new name." Recommendation: start minimal (model name + strategy enum) and grow on demand.
- **Schema-evolution flag semantics** — `--allow-column-removal` and `--allow-full-refresh` interact with the per-model `SchemaEvolutionStrategy` metadata. Phase 2's `should_force_full_refresh` helper must preserve the exact precedence the CLI implements today.
- **`model_compiled` reporter callback shape** — does it fire once per model or once per batch (incremental)? Recommendation: once per *batch* (CLI's `-v` today prints SQL per batch), with a `batch_index: Option<usize>` parameter. UI defaults to ignoring it; CLI's `StdoutReporter` prints all of them when `verbose` is set.
- **End-to-end parity test runtime** — running `examples/web_analytics/` end-to-end takes ~5 seconds against DuckDB. The Phase 5 test will roughly double the CI time for `smelt-runtime`. Acceptable; this is the gate the parity rule promises.
- **`smelt backbuild` will start to diverge** from `smelt run` as `commands/run.rs` migrates and `commands/backbuild.rs` stays behind. Mitigation: Phase 4's `commit` message explicitly notes backbuild as the next plan's target; the divergence is bounded by the fact that both ultimately call into the same compile pipeline.

## Open questions

- **Should `PlanSummary` be public surface or internal?** If user-facing tooling (e.g. an LSP code-action that previews planner decisions) wants to consume it, the type stays `pub`. Current lean: keep it `pub` because `commands/explain.rs` already renders it.
- **Does `enforce_safety = false` log a warning or stay silent?** CLI today logs at `warn!` level. Keeping that means `tracing` configuration is consistent. Recommendation: keep the warning.
- **Does `RunReporter::model_compiled` get the compiled SQL by reference or by value?** By reference (`&str`) is cheaper but ties the reporter's lifetime to the runtime's internal compile state; by value (`String`) costs an allocation per model per batch. Recommendation: by `&str` — reporters that need to keep the string clone it themselves, which is rare.
- **What does Phase 7 do with the existing `smelt-cli` re-exports that aren't from the deleted shims** (e.g. `smelt_cli::find_project_root`, `smelt_cli::Config`)? Those stay — they're stable user-facing CLI library API. Only the runtime-related re-exports leave.

## References

- Predecessor plan: `docs/plans/20260523-smelt-runtime-extraction.md` (phases 0–4a done, 4b–5 partial).
- Predecessor research: `docs/research/20260523-lsp-cli-ui-divergence.md`.
- Existing parity rules (already enforced): `docs/specs/architecture.md` → "Workspace loading parity rule (CLI ↔ LSP)" and "Project isolation rule".
- Spec section being extended: `docs/specs/architecture.md` → "Run pipeline parity rule (CLI ↔ UI)" (gains the structural-enforcement clause in Phase 7).
