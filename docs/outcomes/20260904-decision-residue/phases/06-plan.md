# Phase 6 plan — append-only posture: a late append is an observation, not a violation

## Objective

Split the append-only posture probe's single "violated" verdict into two: a row-count
**increase** in an already-closed partition is a recorded late append that the run reports and
carries on from, while a row-count **decrease** or a fingerprint change at an unchanged count
still fails the run with `SourceMutationProfileViolated`. Advances success criterion 6 (and,
with it, the last consumer-visible clause of §Constraints "Declared lateness is
orchestration-only" — the classification consults what landed, never a declared lateness).

## Planning calls (record in the summary too)

- **The SQL probe keeps the violation leg; a new pure function owns the late-append leg.**
  `emit_append_only_posture_probe` stays the single owner of "is this a posture violation",
  with its predicate narrowed; the late-append set is classified purely in `smelt-logical`
  from the **baseline snapshot the runtime already executes on a held probe**
  (`emit_append_only_baseline_snapshot`), mirroring
  `contract/frozen_horizon.rs::late_arrivals`. No second SQL round trip, no second rendering
  of the same verdict.
- **Only a partition with `check_fingerprint: true` (strictly below the recorded maximum —
  i.e. closed) can produce a late append.** The still-open frontier partition legitimately
  gains rows on every run; reporting that as a late arrival would make the observation noise
  on every single run. Its increase stays silent, exactly as today.
- **A delete-plus-insert that nets to a count increase reads as a late append.** The stored
  baseline is one aggregate fingerprint per partition, so subset-ness is not provable; the
  count leg governs and the spec says so plainly rather than implying a proof smelt cannot
  make.
- **"An observed delta the next run re-processes" is scoped to recording + re-run.** The late
  append is recorded (refreshed baseline, run-manifest `observed`, a `tracing::warn!` line)
  and a catch-up run over that window re-processes it — which is what the conformance
  late-append coverage drives. A scheduler *selecting* that window unprompted is the outcome's
  own "Out of scope" item, unchanged.

## Spec delta (implement first)

- `docs/specs/model_properties.md` §"Probe obligation", the `mutation_profile.kind:
  append_only` row: violation condition becomes "a partition's row count **decreases**, or a
  closed partition's fingerprint changes **at an unchanged row count**"; add that a count
  increase in a closed partition is recorded as a late append (run-manifest `observed`), not a
  violation, and that a net-increase delete+insert is therefore read as a late append.
- `docs/specs/model_properties.md` §Known Divergences: delete the bullet "**The append-only
  posture probe does not yet distinguish a late append from a violation**".
- `docs/specs/sources.md` §Semantics 4 and the `SourceMutationProfileViolated` row (§"Source
  diagnostics"): same narrowing of the violation condition.
- `docs/specs/run_state.md` §"Run manifest": name the append-only probe's `observed` as the
  late-append partition count alongside the existing `retain_departed` example.
- Grep `docs-site/docs/` for user-facing text stating the append-only violation condition and
  update it in the same commit.

## Tests (red first)

Pure classifier — `crates/smelt-logical/tests/append_only_posture_classification.rs` (new):
- `count_increase_in_closed_partition_is_a_late_append` — increase + changed fingerprint on a
  `check_fingerprint: true` partition ⇒ one late append, zero violations, `added_rows` correct.
- `count_decrease_is_a_violation` — decrease ⇒ violation, even with a changed fingerprint.
- `changed_fingerprint_at_equal_count_is_a_violation` — the in-place-update case survives.
- `increase_in_the_open_frontier_partition_is_neither` — `check_fingerprint: false` ⇒ silent.
- `partition_absent_from_baseline_is_neither` — a brand-new partition is an ordinary append.

Emitter — `crates/smelt-logical/tests/probe_execution.rs` (live DuckDB):
- `append_only_posture_probe_ignores_a_pure_late_append` — a closed partition that gained rows
  returns zero violations.
- Update `append_only_posture_probe_returns_nonzero_with_samples_on_violating_data` /
  `emit_statements.rs::append_only_posture_probe_flags_shrunk_partition_and_changed_fingerprint`
  so the fingerprint case holds the row count equal (the shrink case is unaffected).

Runtime end-to-end — `crates/smelt-runtime/tests/source_probes.rs` (real `execute_project`,
DuckDB):
- `late_append_into_closed_partition_does_not_fail_the_run` — the run succeeds, the refreshed
  baseline reflects the new count, and the probe record carries `observed = 1`.
- `deleted_row_in_closed_partition_still_fails_the_run` — `SourceMutationProfileViolated`.
- `in_place_update_in_closed_partition_still_fails_the_run` — ditto.

Conformance — `crates/smelt-cli/tests/maintenance_conformance/probes.rs`:
- `late_append_schedule_holds_with_probes_on` — a generated schedule containing
  `ConformanceStep::AppendLateRow` plus its catch-up rerun, run with the harness's probe
  cadence on, converges to the full-refresh oracle over everything landed with no probe
  failure.

## Tasks

1. Land the spec delta above (spec-first).
2. Add `late_appends(baseline, current) -> Vec<LateAppend { partition_value, added_rows }>` (and
   its `LateAppend` shape) to `smelt-logical`'s maintenance layer, gated on
   `check_fingerprint`; document it as the late-append half of the posture verdict.
3. Narrow `emit_append_only_posture_probe`'s predicate to `current_count < recorded_count OR
   (check_fingerprint AND current_count = recorded_count AND fingerprint IS DISTINCT FROM
   recorded)`; update its doc comment to name the two-verdict split.
4. In `source_probes.rs`, classify the held probe's snapshot rows against the recorded baseline
   via `late_appends`; return them on the dispatch result, `tracing::warn!` one line per source
   naming the partitions, and set `ProbeRecord.observed` to the late-append partition count.
5. Thread the observation through both `execute.rs` dispatch sites onto the run manifest.
6. Flip `render_smelt_yml_for`'s `probes: {cadence: off}` to `per_run` and delete the
   workaround comment that cited this very limitation; add the conformance test above. If a
   pool unrelated to late appends fails under the flip, keep the flip scoped to the
   append-only pool and say so in the summary.
7. Re-check the `partition_residue_probes.rs` ratchet and drop it if this phase closes a
   counted bullet.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test append_only_posture_classification --test emit_statements --test probe_execution --test probe_obligation`
- `cargo test -p smelt-runtime --test source_probes --test statement_parity`
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance`
- `cargo test -p smelt-cli --test example_diagnostics` and `cargo test -p smelt-lsp --test example_workspaces`

## Commit message

`feat(sources): classify a late append into a closed partition as an observation, not an append-only violation`
