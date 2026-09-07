# Phase 6c — Append-only probe dispatch for the succession grain

## Objective

A succession model's declared `mutation_profile: append_only` source posture is
never verified at runtime: `append_only_posture_probes` /
`dispatch_and_record_append_only_postures` are called only from the two ordinary
`match plan.incremental` sites in `crates/smelt-runtime/src/execute/project/mod.rs`,
which the succession dispatch returns before ever reaching, and
`build_succession_run_record` hardcodes `probes: Vec::new()`. Wire the dispatch into
the succession branch (baseline refresh persisted, records threaded into the run
record), then land blocked phase 6's two conformance legs on top. Advances criterion 7
and closes the last runtime gap in criterion 5's probe obligation.

## Spec delta

`docs/specs/incremental_shapes.md` §"Run shape and late events" — append one sentence
(same shape as phase 6b's frontier sentence): a succession run dispatches the source
append-only posture probe before its fold on the same terms as every other maintained
grain, so a late append into a closed partition is an observation whose covering window
is re-presented, while an in-place mutation of a closed partition fails the run with
`SourceMutationProfileViolated` before either the presented table or the ledger is
touched. No other spec section changes (criterion 10's divergence rewrites are phase 10).

## Tests

Runtime, new `crates/smelt-runtime/tests/succession_probes.rs` (copy
`succession_frontiers.rs`'s fixture harness, which appends its own `state:`/`probes:`
block to the copied `smelt.yml` rather than editing the shared fixture):

1. `succession_run_establishes_the_source_posture_baseline` — after one window-forward
   run with `probes.cadence: per_run`, `.smelt/` carries a `SourcePostureStore` entry for
   the model's `append_only` source. RED today (store stays empty).
2. `succession_run_record_carries_the_append_only_probe` — that run's `ModelRunRecord.probes`
   holds one `SourceMutationProfileViolated` / `mutation_profile.kind: append_only` record.
   RED today (`probes: Vec::new()`).
3. `succession_in_place_mutation_of_a_closed_partition_fails_loud` — run 1 establishes the
   baseline over two partitions; `UPDATE` a payload in the strictly-lower (closed) partition;
   the next run errors naming `SourceMutationProfileViolated`, and presented table + tombstone
   ledger are byte-unchanged.
4. `succession_late_append_into_a_closed_partition_is_tolerated` — appending a row to a closed
   partition and re-running its covering window succeeds and refreshes the baseline.
5. `succession_full_rebuild_verifies_the_source_posture_too` — the `--full-refresh`/rebuild arm
   dispatches the same probe (parity with the ordinary full-refresh site at `project/mod.rs`).

Conformance, `crates/smelt-cli/tests/maintenance_conformance/probes.rs` (phase 6's two legs):

6. `succession_late_append_into_a_closed_event_time_partition_re_presents` — a
   `SourceRecipe::succession_events_event_time_partitioned` recipe, two windows driven so the
   first event-time partition is closed, then a late event inserted into it and its covering
   window re-driven: the run holds and `assert_succession_equivalence_for` passes.
7. `succession_in_place_mutation_fails_with_source_mutation_profile_violated` — via
   `drive_succession_window_expect_probe_failure`, assert the returned message names
   `SourceMutationProfileViolated` and does **not** name `SuccessionClockTie` (the probe must
   fire before the fold, not incidentally through the tie probe).

## Tasks

1. Spec delta above (one sentence).
2. New `crates/smelt-runtime/src/maintenance_driver/succession/probes.rs`:
   `dispatch_succession_source_probes(backend, policy, file_store, state_io_lock, model_name,
   cell_label, model_file, source_infos, model_target, schema, dialect) -> Result<Vec<ProbeRecord>>`
   — lifted verbatim from `project/mod.rs`'s window-forward block (load store → build probes →
   dispatch → extend records → `record` refreshed baselines → save), never re-derived.
3. Call it once in the succession branch of `execute/project/mod.rs`, before the
   `if request.full_refresh || force_full_refresh || request.rebuild` split so both arms are
   covered; thread the returned records into `build_succession_run_record`'s new `probes` argument.
4. Keep `crates/smelt-runtime/src/execute/project/mod.rs` **at or under its 4689-line baseline**:
   pay for the new call site by moving the succession dispatch's ~25-line block comment
   (lines 2283–2307) onto `maintenance_driver::succession`'s module docs, where it belongs.
5. Runtime tests 1–5 (red first, then task 2/3 turns them green).
6. Testkit: add an in-place mutation helper to `crates/smelt-maintenance-testkit/src/gate_succession.rs`
   (`mutate_row_payload_in_place_succession(project, recipe, key, event_time, new_payload)` —
   an `UPDATE` that changes a closed partition's fingerprint without changing its row count),
   with its own unit test.
7. Conformance legs 6–7 in `probes.rs`; flip outcome phase row 6 from `blocked` to `done`
   with a one-line note pointing at this phase, and append a Decision-log line.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test succession_probes --test succession_frontiers --test technique_lowering --quiet`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --quiet`
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` (full seeded sample)
- `cargo test -p smelt-maintenance-testkit --quiet`
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`feat(succession): verify the append-only source posture on every succession run`
