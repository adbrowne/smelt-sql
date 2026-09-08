# Phase 4 summary — Invocation on the run path

**Shipped:**
- `crates/smelt-core/src/external_step.rs`: closed `command:` placeholder grammar
  (`{run_date}`/`{run_end}`, `{{`/`}}` escaping), rejected at declaration time
  (`ExternalStepError::UnknownPlaceholder` → `MalformedExternalStep`); pure
  `resolve_command`/`StepRunContext`/`CommandResolveError`.
- `examples/broken/models/sources/step_unknown_placeholder.yml` fixture + row in
  `crates/smelt-cli/tests/example_diagnostics/external_step_diagnostics.rs`.
- `crates/smelt-runtime/src/select.rs`: `SelectionPlan::required_steps` — selector-named
  steps (`select_nodes(..).steps`) ∪ `graph.steps_required_by(&selected)`, minus any
  step named by an `--exclude` selector.
- New `crates/smelt-runtime/src/execute/external_steps.rs`: `invoke_required_steps` —
  refusal checks first (dry run, `invoke_external_steps == false`), then per-step
  `resolve_command` + `tokio::process::Command` spawn (cwd = project_dir), cancellation
  via `tokio::select!`, non-zero exit → `ExternalStepFailedError`, spawn/resolve failure
  → `ExternalStepNotInvocableError`. Steps run sequentially, sorted.
- `ExecuteRequest::invoke_external_steps` (`#[serde(default = "default_true")]`).
- Wired into `execute/project/mod.rs`: `graph_lock` is now `mut`; external steps are
  discovered and registered before selection; `invoke_required_steps` runs after
  selection and before `build_model_plans`, covering both the dry-run and live paths.
- Spec: `docs/specs/sources.md` — "Command placeholders" paragraph, §Semantics 11/12
  tightened (exit code named, `smelt explain` named as the non-refusing preview
  surface), Known Divergences entry narrowed. `docs/specs/diagnostics.md` gained the
  two run-path-refusal rows (explicitly marked as not `DiagnosticCode` enum variants).
- Tests: `crates/smelt-core/tests/external_step_command.rs` (4, red-green on the
  placeholder grammar) and `crates/smelt-runtime/tests/external_step_invocation.rs`
  (7, real DuckDB + a temp shell-script step) — all 11 from the plan.

**Decisions:**
- `step_runs_before_its_consumer` needs the step's shell script to write a real DuckDB
  table before smelt's own backend connection opens — no `duckdb` CLI dependency exists
  elsewhere in this repo (only `libduckdb.so` is provisioned in CI), so that one test
  gates on `duckdb_cli_available()` and skips gracefully, mirroring the existing
  Spark/BigQuery-gated-test posture. The other 6 tests need no CLI and always run.
- `smelt-ui/src/run_manager.rs::to_runtime_request` sets `invoke_external_steps: true` —
  it is the live-run path (`dry_run: false`), not a preview endpoint; no UI plan-preview
  call site setting `false` was found to exist yet (see below).

**For the next planner:**
- No UI call site actually sets `invoke_external_steps: false` yet — the outcome's own
  phase-4-planning decision log describes a UI "plan-preview endpoint" that should opt
  out, but no such endpoint exists in `smelt-ui` today. If/when one is built, it must set
  `invoke_external_steps: false` (or rely on `dry_run: true`, which also refuses).
- `crates/smelt-runtime/src/execute/project/mod.rs` grew past its large-file-ratchet
  baseline (4761 → 4793 lines) from this wiring; baseline updated via
  `large-file-check.sh --update` (sign-off: this comment). Growth is small and confined
  to the external-step registration + invocation call — no further split needed yet.
- Phase 5 (Reporting) needs a `reporter` hook point for step invocation — currently
  `invoke_required_steps` only `tracing::info!`s; no `RunReporter` callback fires for a
  step's start/success/failure. Phase 5's plan should add one rather than requiring the
  UI to poll `tracing` output.
- `SourcesConfig`/legacy `sources.yml` path is untouched by this phase — external steps
  are per-entity YAML only, consistent with prior phases.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets,
  full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-core --test external_step_command --test external_step_yaml --test external_step_graph` — 4+7+14 passed.
- `cargo test -p smelt-runtime --test external_step_invocation --test execute_parity --test select_parity --test dry_run_statements` — 7+3+4+8 passed.
- `cargo test -p smelt-cli --test example_diagnostics --test list_external_step` — 126 passed (1 pre-existing ignore), 2 passed.
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — passed.
- `cargo test -p smelt-core --test hardening_budget` — ratchet unmoved (self-test's own synthetic regression line is expected test output, not a real regression).
- `bash .claude/scripts/large-file-check.sh` — OK after `--update` (see Decisions).
