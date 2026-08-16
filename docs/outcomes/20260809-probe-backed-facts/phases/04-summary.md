# Phase 4 summary — probe policy: `probes:` in `Config`, cadence decision, shared dispatch + named firing diagnostic

## Shipped

- `probes:` is a real `Config` field (`crates/smelt-core/src/config.rs`): `ProbesConfig { cadence: ProbeCadence }`,
  `ProbeCadence::{PerRun, Periodic { every_n_runs: u32 }, Off}`. Custom `Deserialize` cross-validates
  `cadence: periodic` against a sibling `periodic.every_n_runs` block, failing loud (never a silent
  default) on a missing block or `every_n_runs: 0`; `deny_unknown_fields` throughout.
- Pure cadence decision: `crates/smelt-logical/src/maintenance/probe_cadence.rs`,
  `should_dispatch(cadence, run_ordinal) -> ProbeDispatch::{Dispatch, Skip(SkipReason)}`, re-exported
  from `smelt_logical::maintenance`. `SkipReason::{CadenceOff, NotThisPeriod}`.
- Single-owner dispatch helper: `crates/smelt-runtime/src/probes.rs` — `ProbePolicy` (cadence +
  run ordinal), `ProbeContext` (fact, probe code, model, licensed cell, remedy), `dispatch_probe`
  (consults cadence, executes SQL, parses the shared `violation_count`/`sample_keys` contract,
  fails loud on a malformed row), `probe_violation_suffix` (shared cell+remedy trailer every
  firing message appends).
- Both already-dispatched probes re-routed through the new policy:
  - Recurrence-bound probe (`run_windowed_keyed_maintenance`) now calls `dispatch_probe` directly
    — its row shape already matches the shared contract.
  - Count-preservation probe (`execute_delete_insert_with_delta_restriction`) keeps its own
    `driving_count`/`enriched_count` row parsing (a different shape statement_parity's golden SQL
    text locks in) but now consults `should_dispatch` with the same policy before running, and
    appends `probe_violation_suffix` to its existing error text on violation.
  - Both functions gained a `probe_policy: &ProbePolicy` parameter; `execute.rs` builds one per
    model per dispatch call from `config.probes.cadence` and the model's prior-run count
    (`smelt_state::history::HistoryQuery::for_model` over one `file_store.load_runs(None)` load
    per run, shared across models via the run's existing by-reference rebinding block).
- `ModelRunRecord.probes: Vec<ProbeRecord>` (`smelt-state`), `ProbeRecord { fact, probe, outcome:
  Dispatched | Skipped }`, `#[serde(default)]`-defaulted empty — round-trips legacy manifests.
- Spec: `smelt_yml.md` drops the "unimplemented" divergence and states the skip-records-unverified
  rule; `model_properties.md` §"Probe cadence" states the policy-skip-vs-unbuildable distinction
  and the firing-diagnostic shape; `run_state.md` §"Run manifest" documents the new `probes` array.

## Decisions

- The count-preservation probe's row shape (`driving_count`/`enriched_count`) does not match the
  shared `violation_count`/`sample_keys` contract `dispatch_probe` parses, and its SQL text is
  locked by `statement_parity`'s golden assertions. Rather than reshape its emitted SQL (out of
  this phase's scope — task list names only `smelt-core`/`smelt-logical`/`smelt-runtime`/
  `smelt-state` changes), that call site consults `should_dispatch` directly for the cadence
  decision and reuses only `probe_violation_suffix` for the shared cell+remedy trailer.
  `dispatch_probe` itself stays the generic executor for probes that do speak the shared contract
  (recurrence-bound today; the four phase-2 probes in phases 5–6).
- `ModelRunRecord.probes` population (plan task 6, "record each probe's dispatched/skipped outcome")
  is **not wired end-to-end** — the field exists, defaults correctly, and round-trips, but no
  dispatch site currently pushes an entry into it. Wiring this requires either a shared mutable
  log threaded through `ProbePolicy` (adding a field, not a new call-site parameter — the ~30
  call sites already updated for `probe_policy: &ProbePolicy` would not need to change again) or
  returning probe outcomes alongside `ExecutionResult`/`StatementGroup` back to `execute.rs`'s
  several separate `ModelRunRecord` construction sites. Deferred — see below.

## For the next planner

- **Follow-up, not blocking**: wire `ModelRunRecord.probes` population. Recommended shape: add
  `log: Arc<Mutex<Vec<smelt_state::ProbeRecord>>>` to `ProbePolicy` (constructed fresh per model in
  `probe_policy_for_model`), have `dispatch_probe` and the count-preservation call site push into
  it, and read it back via a `take_records()` method at each of `execute.rs`'s `ModelRunRecord`
  construction sites for that model. This closes the loop for `smelt explain` (phase 8) to report
  per-run dispatch/skip status.
- Phases 5–6 (live dispatch of the four phase-2 probes + the append-only posture probe) can now
  call `dispatch_probe` directly — it already speaks their `violation_count`/`sample_keys`
  contract; no further plumbing needed on the dispatch-helper side.
- `docs/specs/model_properties.md`'s registry rows for the four phase-2 probes still read
  `built (unwired)` — phases 5–6 are the ones that flip them to `built`, not this phase (only the
  two pre-existing rows, `key_recurrence` and `referential_integrity`, moved from "wired, no
  cadence control" to "wired, cadence-governed, named diagnostic" here).

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full `cargo test`,
  example_diagnostics).
- `cargo test -p smelt-logical --test probe_cadence --test probe_obligation` — 3 + 6 passed.
- `cargo test -p smelt-runtime --test probe_dispatch --test locality_route3_recurrence_check --test technique_lowering --test statement_parity` — 6 + 3 + 21 + 30 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --test example_diagnostics` — 119 (+1 ignored) + 59 passed.
- `cargo test -p smelt-lsp --test example_workspaces` — 34 passed (extra check beyond the plan's list).
