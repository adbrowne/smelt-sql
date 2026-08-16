# Phase 3 plan — absent-state runtime behaviours

## Objective

Make the runtime match the absent-state sentences phase 1 landed (criterion 4's second half):
an absent source-posture baseline and an absent frozen-band baseline both *report*
`ProbeBaselineUnavailable` instead of silently establishing, and an absent schema snapshot
degrades to "new" with a say-so when the posture is what excluded it. Task 1 first clears the
second pre-existing red-gate class phase 2 found, so criterion 6 has no standing gate red
underneath the rest of this outcome.

## Spec delta (make these edits first)

1. **`docs/specs/run_state.md` §"Run manifest"** — the probe-record `outcome` vocabulary becomes
   `"dispatched" | "skipped" | "baseline_established"`; add one sentence: a
   `baseline_established` entry means the probe found no recorded baseline to compare against,
   established one from the current observation, and reported `ProbeBaselineUnavailable`
   (`state.md` §"Diagnostics") — the declaration was neither verified nor disproved this run.
   Update the inline JSON example's comment on line 71/76 accordingly.
2. **`docs/specs/incremental_models.md`** (task 1 only, and only if the judgment lands
   spec-side) — see task 1.

No other spec edits. `state.md`, `sources.md`, `schema_evolution.md` already say what this phase
implements; do not restate or re-word them.

## Tests (red-green, in this order)

Task-1 gates (already red on this branch — turn them green, do not weaken the assertion):
- `smelt-logical --test output_delta_spec graph_layer_states_typed_edges_and_narrowed_refusal`
- `smelt-logical --test typed_edge_spec typed_edges_section_names_the_three_component_parts`

New:
- `smelt-state` unit (`lib.rs` probe-record tests) — `probe_record_serde_round_trips_baseline_established`:
  the new outcome serialises as `"baseline_established"` and an older manifest without the field
  still defaults to an empty `probes` list.
- `smelt-runtime/tests/source_probes.rs` — `first_observation_records_baseline_established_and_reports_advisory`:
  an `append_only` source with no recorded baseline yields a `baseline_established` probe record
  and one reporter advisory naming `ProbeBaselineUnavailable`, the source address, and why the
  baseline was absent.
- `smelt-runtime/tests/source_probes.rs` — `second_run_verifies_and_reports_no_advisory`:
  the follow-up run compares against the baseline, records `dispatched`, emits no advisory
  (guards against advisory-on-every-run).
- `smelt-runtime/tests/state_posture.rs` — `stateless_posture_reports_baseline_unavailable_every_run`:
  under `state.mode: stateless` every run is an establishing run and each emits the advisory —
  the "reported, not silent" half of `sources.md` §Semantics 4.
- `smelt-runtime/tests/contract_late_arrival_probe.rs` — `frozen_band_first_observation_reports_baseline_unavailable`:
  the absent-frozen-band-baseline branch records `baseline_established` + advisory, and does not
  produce a `LateArrivalViolation`.
- `smelt-runtime/tests/contract_late_arrival_probe.rs` — `deferral_never_reports_baseline_unavailable`:
  guard for the phase-1 asymmetry — `contract.deferral` does not degrade (its refusal is phase 5's
  `DeclaredContractRequiresState`, not an advisory here).
- `smelt-cli/tests/e2e` (new file `absent_schema_snapshot.rs`) —
  `diff_reports_new_and_says_state_excluded_under_stateless`: with `state.mode: stateless`,
  `smelt diff` reports every model `new` **and** prints one line saying the deployed-schema
  snapshot is absent because the posture excludes it; and
  `diff_reports_new_after_snapshot_deleted`: deleting `.smelt/schemas/<model>.json` under
  `intervals` reports `new` without refusing.

## Tasks

1. Read `docs/specs/incremental_models.md` §"The graph layer" (the *second* heading, ~line 1258),
   `crates/smelt-logical/tests/{output_delta_spec,typed_edge_spec}.rs` in full. Repoint both
   heading lookups so they cannot match the Overview mention (match the section by a
   disambiguating anchor, not first-`###`-wins). Then decide the `General` question with intent:
   if the graph layer's keyed-node refusal really is scoped to the output-delta profile verdict
   `General`, the *spec* prose should name it (spec edit); if the prose's lowercase `general`
   correctly names the delta-signature verdict, the *test's* expectation is stale (test edit).
   Record the decision and its reasoning in the phase summary either way.
2. `smelt-state`: add `ProbeRecordOutcome::BaselineEstablished` (serde
   `baseline_established`), doc-cited to `run_state.md` §"Run manifest".
3. `smelt-runtime/src/reporter.rs`: add one default-no-op hook —
   `fn probe_advisory(&self, _run_id: &str, _model: &str, _code: &str, _message: &str) {}` —
   doc-cited to `state.md` §"The optionality rule" (degradation must be reported even where no
   manifest is written, i.e. under `stateless`).
4. `source_probes.rs`: the `SourcePostureAction::Establish` dispatch arm records
   `BaselineEstablished` and calls `probe_advisory` with `ProbeBaselineUnavailable` and a message
   naming the source, the partition set, and the reason (no recorded baseline / posture excludes
   baselines). Thread the reporter into `dispatch_and_record_append_only_postures`.
5. `contract_probes.rs`: same for the `recorded.is_empty()` branch of
   `dispatch_and_record_frozen_horizon_probes`. Leave `evaluate_deferral` untouched.
6. `smelt-cli` reporter impl: print the advisory as a warning line (`ProbeBaselineUnavailable: …`);
   `smelt-ui`'s reporter inherits the default no-op unless it already surfaces warnings.
7. `smelt-cli/src/commands/diff.rs`: when `load_schema` returns `None` **and**
   `config.state.mode` excludes schema snapshots, add the say-so line alongside the `new` status
   (both text and `--json` output; JSON gets a `"snapshot_absent_reason"` field). Absent-because-
   deleted under a snapshot-writing posture keeps today's plain `new`.
8. Update `docs/specs/run_state.md` per the spec delta if not already done in step 0, and
   re-check `rg -n 'baseline_established' docs/specs/run_state.md` resolves.

## Verification

- `bash .claude/scripts/verify-phase.sh` (full) — must be fully green; the two gates from task 1
  are the reason this phase exists first, so a red there is a phase failure, not a pre-existing
  excuse. Confirm no *new* pre-existing-red class is being carried forward; if one appears,
  `git stash`-confirm it and record it in the summary.
- `cargo test -p smelt-logical --test output_delta_spec --test typed_edge_spec`
- `cargo test -p smelt-runtime --test source_probes --test state_posture --test contract_late_arrival_probe --test probe_manifest`
- `cargo test -p smelt-state`
- `cargo test -p smelt-cli --test e2e absent_schema_snapshot`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `git diff --stat -- docs/specs/` — only `run_state.md` (plus `incremental_models.md` iff task 1's
  judgment landed spec-side).

## Commit message

`feat(state): report ProbeBaselineUnavailable for absent probe baselines and absent schema snapshots`
