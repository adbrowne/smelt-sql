# Phase 4 — Invocation on the run path

## Objective

Make a declared external step actually *run*: ordered ahead of every consumer of the
sources it produces, invoked by `execute_project`, with a non-zero exit propagated as a
run failure naming the step and its exit code (downstream models unbuilt), and a named
refusal when the run may not invoke it. Advances success criterion 4 and the ordering leg
of criterion 3. Also settles the `command:` placeholder grammar the phase-2 planner folded
into this row (criterion 1's "decided before code" clause).

## Spec delta (first)

`docs/specs/sources.md` §"Externally-produced sources (black-box steps)":

- New paragraph after the key table — **command placeholders**. Substitution is
  per-argv-element, over a closed placeholder set: `{run_date}` (run-window start, ISO
  `YYYY-MM-DD`) and `{run_end}` (exclusive end, same form). `{{` / `}}` are literal
  braces. Any other `{name}` is `MalformedExternalStep` at declaration time (so the LSP
  shows it, never the run). A placeholder whose value this run does not have (a run with
  no window) is `ExternalStepNotInvocable`.
- §Semantics 12 — name the refusal conditions concretely: a dry run; a run whose
  environment declines external invocation; a `command:` that cannot be spawned (not
  found / not executable); a placeholder with no value in this run. State that
  `smelt explain` — not `--dry-run` — is the non-refusing preview surface for a step.
- §Semantics 11 — the failure names the step address *and* its exit code; downstream
  models are unbuilt because required steps run to completion before any model executes.
- §Known Divergences — narrow the external-step entry to "invoked on the run path;
  `smelt explain` rendering unbuilt".

`docs/specs/diagnostics.md` — two catalogue rows for `ExternalStepNotInvocable` and
`ExternalStepFailed`, marked as run-path refusals (not `DiagnosticCode` enum variants,
so the `diagnostics_catalogue` gate is unaffected — it runs enum → catalogue only).

## Tests (red first)

`crates/smelt-core/tests/external_step_command.rs` (new):
1. `run_date_placeholder_substituted` — `{run_date}` in one argv element becomes the
   window start; other elements untouched.
2. `escaped_braces_are_literal` — `{{run_date}}` resolves to the literal `{run_date}`.
3. `unknown_placeholder_is_malformed` — `{nope}` fails `parse_external_step_yaml`
   (→ `MalformedExternalStep`), not resolution.
4. `placeholder_without_window_is_not_invocable` — resolving `{run_date}` with no window
   yields the not-invocable error, distinct from the failed-exit error.

`crates/smelt-runtime/tests/external_step_invocation.rs` (new; real DuckDB, step command
is a temp shell script):
5. `step_runs_before_its_consumer` — script lands the produced source's table; the
   consumer model builds from it (ordering proven by the run succeeding at all).
6. `nonzero_exit_fails_run_naming_step` — error text contains `ExternalStepFailed`, the
   step address and the exit code; the consumer's table does not exist.
7. `unspawnable_command_refuses` — a `command:` naming no executable →
   `ExternalStepNotInvocable`; nothing built.
8. `dry_run_reaching_step_refuses` — `dry_run: true` over a selection reaching the step →
   `ExternalStepNotInvocable`.
9. `environment_declining_invocation_refuses` — `invoke_external_steps: false` → same code.
10. `unreached_step_is_not_invoked` — a step whose produced sources no selected model
    reads leaves its marker file absent and the run green.
11. `selecting_the_step_alone_invokes_it_and_builds_no_models` — `--select <step>` runs
    the command, builds zero models.

## Tasks

1. Land the spec deltas above (sources.md, diagnostics.md).
2. `smelt-core/src/external_step.rs`: closed placeholder set + parse-time rejection of an
   unknown one (new `ExternalStepError` variant mapping to `MalformedExternalStep`); pure
   `resolve_command(&ExternalStepInfo, &StepRunContext) -> Result<Vec<String>, CommandResolveError>`
   with `StepRunContext { run_date: Option<String>, run_end: Option<String> }`.
3. `examples/broken/` fixture for the unknown placeholder + its row in
   `crates/smelt-cli/tests/example_diagnostics/external_step_diagnostics.rs`.
4. `smelt-runtime/src/select.rs`: `SelectionPlan` gains `required_steps: Vec<String>` =
   selector-named steps (`select_nodes(..).steps`) ∪ `graph.steps_required_by(&selected)`,
   minus any step named by an `--exclude` selector. Deterministic (sorted).
5. New `crates/smelt-runtime/src/execute/external_steps.rs`: `invoke_required_steps(...)`
   — refusal checks first (dry run, `invoke_external_steps == false`), then per step
   resolve argv, spawn via `tokio::process::Command` with cwd = `project_dir`, honour the
   `CancellationToken`, map a spawn error to `ExternalStepNotInvocable` and a non-zero
   exit to `ExternalStepFailed`. Steps run sequentially in sorted order; `tracing::info!`
   per step (reporter hooks are phase 5's).
6. `ExecuteRequest` gains `#[serde(default = "default_true")] pub invoke_external_steps: bool`
   — the "environment that cannot execute" leg. No new CLI flag this phase.
7. Wire into `execute/project/mod.rs`: `graph_lock` becomes `mut`, `discover_external_steps`
   + `add_external_steps` (idempotent map inserts; mirrors the existing inline
   `discover_source_infos` precedent), then `invoke_required_steps` after selection and
   before `build_model_plans` — including on the `dry_run` branch, ahead of
   `build_dry_run_outcome`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-core --test external_step_command --test external_step_yaml --test external_step_graph`
- `cargo test -p smelt-runtime --test external_step_invocation --test execute_parity --test select_parity --test dry_run_statements`
- `cargo test -p smelt-cli --test example_diagnostics --test list_external_step`
- `cargo test -p smelt-db --test integration diagnostics_catalogue`
- `cargo test -p smelt-core --test hardening_budget` (ratchet unmoved)
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`feat(sources): invoke external steps on the run path`
