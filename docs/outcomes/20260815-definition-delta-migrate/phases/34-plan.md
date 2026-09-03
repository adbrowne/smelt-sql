# Phase 34 — Persist the `retain_departed` probe outcome on the run manifest

## Objective

The snapshot-reconcile path already dispatches the declared point's anti-join probe
(`cumulative.rs`'s `DepartedKeyDisposition::Retain` arm) but only `tracing::info!`s the
retained-departed count and fails loud inline on an unmarked tombstone; the cumulative arm's
`ModelRunRecord` still writes `probes: Vec::new()`. This phase threads that probe through the
same `ProbeRecord` ledger the `frozen_horizon`/`deferral` probes use, and lists it in
`smelt explain`'s probe plan — closing the gap between what `incremental_models.md` §Retention
already promises ("recording the retained-departed key count on the run manifest") and what a
run actually persists. Advances the outcome's contract-lattice-point criteria (a declared point
is observable, not just enforced).

## Spec delta (spec-first; the implement step makes these edits)

1. `docs/specs/run_state.md` §"Run manifest" — a `probes[]` entry gains an optional numeric
   `observed`: the probe's recorded scalar measurement, omitted when the probe records no
   count. Show it on the `retain_departed` example line in the JSON block and describe it in
   the prose paragraph (absent on older manifests, absent for probes with no measurement).
2. `docs/specs/incremental_models.md` §"Retention (`retain_departed`)" — state that the probe
   is dispatched on **every** reconcile that suppresses the delete, independent of the
   project's `probes:` cadence (it stands in for the delete the default point would have run,
   so a cadence skip would suppress the delete while verifying nothing); that its manifest
   record carries fact `contract.retain_departed` with the retained-departed count in
   `observed`; and that an unmarked tombstone raises the named
   `ContractDepartedKeyUnmarked`. Add the row to that spec's diagnostics table.
3. `docs/specs/diagnostics.md` — catalogue `ContractDepartedKeyUnmarked` (Error, runtime probe,
   no `DiagnosticCode` variant, like its two siblings), and correct the now-stale sentence in
   the contract-lattice note claiming `retain_departed`'s probe "is not yet dispatched by any
   live run" (phase 32b dispatched it; this phase records it).
4. `docs/specs/cli.md` §`smelt explain` probe report — only if that section enumerates the
   facts explain lists; if it describes them generically, leave it unedited and say so.

## Tests

1. `smelt-state` (`src/lib.rs` tests) `probe_record_observed_defaults_absent` — a manifest JSON
   `probes[]` entry with no `observed` key deserializes to `None`, and a `None` re-serializes
   without the key (no manifest churn for the probes that record no measurement).
2. `smelt-runtime/tests/departed_key_reconcile.rs`
   `retain_departed_probe_is_recorded_with_the_retained_count` — a declared-`retain_departed`
   reconcile whose source drops N keys yields a `ProbeRecord { fact:
   "contract.retain_departed", probe: "ContractDepartedKeyUnmarked", outcome: Dispatched,
   observed: Some(N) }`.
3. same file, `default_point_records_no_probe` — an undeclared model's reconcile (delete leg)
   records no probe at all, so the default point stays measurement-free.
4. same file, `retain_departed_probe_reaches_the_run_manifest` — driven through
   `execute_project`, the model's `ModelRunRecord.probes` in the written manifest carries the
   record (the actual "run-report visible" claim; the cumulative arm's `probes: Vec::new()` is
   what this kills).
5. `smelt-runtime/tests/` probe-plan coverage (extend the existing probe-plan test file, or
   `diagnostics.rs` if none) `probe_plan_lists_declared_retain_departed` — `probe_plan_for_model`
   emits one entry for a model declaring `contract.retain_departed`, none when undeclared.
6. same as 2's file, `unmarked_tombstone_error_names_the_diagnostic` — the unmarked-tombstone
   failure text contains `ContractDepartedKeyUnmarked`.
7. `smelt-logical/tests/contract_lattice_spec.rs` — extend its catalogue assertion to include
   `ContractDepartedKeyUnmarked` so diagnostics.md coverage is gated, not incidental.

## Tasks

1. Land the spec deltas above (1–4) before touching code.
2. `smelt-state`: add `observed: Option<u64>` to `ProbeRecord` with `#[serde(default,
   skip_serializing_if = "Option::is_none")]`; fix the existing construction sites.
3. `smelt-runtime/src/cumulative.rs`: give `execute_snapshot_reconcile` a
   `probe_sink: &mut Vec<smelt_state::ProbeRecord>` out-parameter (the call site is a match arm
   that must keep returning `Result<ExecutionResult>`), push the record in the `Retain` arm
   with the parsed `retained_departed_count`, and document in the doc comment why this probe
   is cadence-independent.
4. Route the unmarked-tombstone `anyhow::ensure!` message through the named
   `ContractDepartedKeyUnmarked` code.
5. `smelt-runtime/src/execute.rs`: declare the sink beside the keyed dispatch, pass it into the
   snapshot-reconcile arm, and feed it into the cumulative arm's `ModelRunRecord.probes`,
   replacing the `Vec::new()` and its now-false "dispatches no declared-fact probes" comment.
6. `smelt-runtime/src/probe_plan.rs`: append a `ProbePlanEntry` for a declared
   `contract.retain_departed` (fact `contract.retain_departed`, probe
   `ContractDepartedKeyUnmarked`, cell `{schema}.{table} reconcile anti-join`).
7. Cross-check no other manifest-writing arm can reach `execute_snapshot_reconcile` with a
   dropped sink; if one does, thread it there too rather than leaving a silent drop.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test departed_key_reconcile`
- `cargo test -p smelt-runtime --test statement_parity`
- `cargo test -p smelt-logical --test contract_lattice_spec`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `rg -n 'not yet dispatched by any live run' docs/specs/` returns nothing

## Commit message

`feat(incremental): the retain_departed reconcile probe is recorded on the run manifest`
