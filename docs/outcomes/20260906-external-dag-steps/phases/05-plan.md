# Phase 5 — Run-path reporting for external steps

**Outcome:** `docs/outcomes/20260906-external-dag-steps/outcome.md`
**Spec anchors:** `docs/specs/run_state.md` §"Run manifest", §"Run report";
`docs/specs/sources.md` §"Externally-produced sources (black-box steps)"

## Objective

Make a step's invocation observable. `invoke_required_steps` today only
`tracing::info!`s: no `RunReporter` event fires, and the run manifest records nothing
about a step the run ran. This phase adds the reporter callbacks, prints them from the
CLI, and records every successfully-invoked step in the manifest (and hence the derived
report artifact). Advances success criteria 4 (failure is a visible run failure) and 5
(the run-report half; `smelt explain` is phase 6).

## Spec delta (first, before code)

- `docs/specs/run_state.md` §"Run manifest": add an optional top-level `external_steps`
  map to the manifest JSON sketch — per step address: `command` (the *resolved* argv,
  after placeholder substitution), `produces` (source addresses), `duration_ms`,
  `outcome: "success"`. Note that only invoked-and-succeeded steps appear.
- `docs/specs/run_state.md` §"Run report": add the same entries, derived (report stays a
  pure function of the manifest). Add one sentence stating the rule the decision log
  settles: a run that fails **during** the step pass aborts before any manifest exists —
  as any pre-execution failure does — so its report is absent, and the step failure is
  surfaced by the run's own output naming the step and its exit code.
- `docs/specs/sources.md` §Semantics 11: append that a step's start, success and non-zero
  exit are each reported to the run's reporter (so an embedder sees them without parsing
  logs), and a succeeded step is recorded in the run manifest. Leave the §Known
  Divergences `smelt explain` sentence in place — phase 6 removes it.

## Tests (red-green)

New `crates/smelt-runtime/tests/external_step_reporting.rs` (recording reporter over the
fixture shape in `tests/external_step_invocation.rs`):

1. `reporter_sees_started_then_completed` — a succeeding step fires
   `external_step_started` (step address + resolved argv) then `external_step_completed`.
2. `reporter_sees_failed_with_exit_code` — a step exiting `3` fires
   `external_step_failed` with `exit_code == 3` and no `external_step_completed`.
3. `refusal_fires_no_step_events` — a dry run refuses (`ExternalStepNotInvocable`) with
   zero step events: nothing was spawned, so nothing may be reported as started.
4. `no_step_events_when_selection_requires_none` — a run selecting a step-free model
   fires no step events (guards spurious emission).
5. `manifest_records_invoked_step` — after a successful run whose selection reaches a
   step, the run manifest's `external_steps` carries the address, the *resolved* argv
   (placeholders substituted), `produces`, and `outcome: success`.
6. `manifest_has_no_external_steps_key_when_none_ran` — the field is omitted (not an
   empty object) for a step-free run, so existing manifests round-trip unchanged.

New/extended in `crates/smelt-state` unit tests:

7. `report_from_manifest_carries_external_steps` — `RunReport::from_manifest` copies the
   step entries; pure, no backend.
8. `manifest_without_external_steps_deserializes` — a manifest JSON predating the field
   loads with an empty map (`#[serde(default)]`), no error.

## Tasks

1. Land the spec delta above (three files' sections; timeless-oracle rule).
2. `smelt-state/src/lib.rs`: `ExternalStepRunRecord { command: Vec<String>, produces:
   Vec<String>, duration_ms: u64, outcome: RunOutcomeKind }`; add
   `RunManifest.external_steps: BTreeMap<String, ExternalStepRunRecord>` with
   `#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]`; mirror onto
   `RunReport` and copy it in `from_manifest`.
3. `smelt-runtime/src/reporter.rs`: three defaulted trait methods —
   `external_step_started(run_id, step, argv)`, `external_step_completed(run_id, step,
   duration)`, `external_step_failed(run_id, step, exit_code, error)` — each documented
   with the spec section it serves.
4. `execute/external_steps.rs`: take `&dyn RunReporter`, fire the three events around the
   spawn, and return `Vec<(String, ExternalStepRunRecord)>` for the steps that succeeded.
   Refusal branches return before any event fires.
5. `execute/project/mod.rs`: bind the returned records at the call site (~line 111) and
   insert them into the `RunManifest` literal (~line 787). No reordering of the step pass.
6. `smelt-cli/src/reporter.rs`: implement the three callbacks on `CliReporter` — a
   `→ step <addr>` line with the argv under `--verbose`, a completion line with duration,
   and a failure line naming the exit code. (`smelt-ui`'s reporter inherits the no-op
   defaults; wiring the UI event stream is not this phase's work.)
7. Update `crates/smelt-runtime/src/execute/external_steps.rs`'s module doc to name the
   reporting contract.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test external_step_reporting --test external_step_invocation --test execute_parity`
- `cargo test -p smelt-state`
- `cargo test -p smelt-cli --test run_report --test cli_docs_coverage`
- `cargo test -p smelt-core --test hardening_budget` (ratchet unmoved)
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`feat(sources): report and record external-step invocation on the run path`
