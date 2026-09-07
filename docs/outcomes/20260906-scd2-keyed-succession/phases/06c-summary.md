# Phase 6c summary — Append-only probe dispatch for the succession grain

## Shipped

- `crates/smelt-runtime/src/maintenance_driver/succession/probes.rs` (new):
  `dispatch_succession_source_probes`, lifted verbatim from the ordinary
  `plan.incremental` dispatch sites in `execute/project/mod.rs`.
- Wired once in the succession branch of `execute/project/mod.rs`, before
  the `request.full_refresh || force_full_refresh || request.rebuild` split
  — both the full-ledger rebuild arm and the window-forward arm now verify
  the driving source's append-only posture before writing.
- `build_succession_run_record` (`frontier.rs`) now takes the accumulated
  `Vec<ProbeRecord>` instead of hardcoding `probes: Vec::new()`.
- `execute/project/mod.rs`'s own ~25-line succession-dispatch block comment
  moved onto `maintenance_driver::succession`'s module docs (`mod.rs`), to
  pay for the new call site and stay at the file's 4689-line large-file
  baseline (ended at 4687).
- New `crates/smelt-runtime/tests/succession_probes.rs` (5 tests): baseline
  establishment, `ModelRunRecord.probes` population, in-place-mutation
  refusal (naming `SourceMutationProfileViolated`, not `SuccessionClockTie`,
  with both the presented table and tombstone ledger byte-unchanged),
  late-append tolerance with a refreshed baseline count, and full-rebuild
  parity.
- New testkit helper `mutate_row_payload_in_place_succession` in
  `crates/smelt-maintenance-testkit/src/gate_succession.rs`, plus its own
  unit test.
- Two conformance legs in `crates/smelt-cli/tests/maintenance_conformance/
  probes.rs`: `succession_late_append_into_a_closed_event_time_partition_re_presents`
  and `succession_in_place_mutation_fails_with_source_mutation_profile_violated`.
- Spec delta: `docs/specs/incremental_shapes.md` §"Run shape and late
  events" now states the probe-dispatch-before-fold guarantee explicitly.
- Outcome phase 6 flipped `blocked` → `done` (subsumed by this phase); 6c
  flipped `planned` → `done`.

## Decisions

- Dispatch site is placed BEFORE the full-refresh/rebuild vs. window-forward
  split so one call site covers both arms — matches the plan's parity
  requirement rather than duplicating the call in each arm.
- Used literal-SQL formatting (not parameter binding) for the testkit's
  in-place `UPDATE`, matching every other statement-building helper in
  `gate_succession.rs`/`insert_row_succession_for`. An earlier attempt with
  `duckdb::params![...]` bound a `TIMESTAMP` column against a `String`
  parameter and silently matched zero rows.
- Discovered mid-implementation: a `duckdb::Connection` opened BEFORE a
  mutation and reused for a post-mutation read-back can return stale
  content (worked in DuckDB's per-connection snapshot semantics) even
  though the row count read from the same stale connection is unaffected
  (row count is the same either way). Fixed by opening a fresh connection
  for every post-mutation read. Documented in the testkit unit test only —
  not spec-worthy, but worth a note for the next test author reusing this
  harness shape.

## For the next planner

- Nothing deferred out of this phase's own scope. Everything in the plan's
  Tasks/Tests lists shipped.
- Phase 6's blocked note's original scenario (a mutation re-driving the
  SAME window failing with `SuccessionClockTie` instead of
  `SourceMutationProfileViolated`) is now moot — the posture probe fires
  before the fold, so the mutation is caught first regardless of which
  window is re-driven.
- Remaining phases (8 explain surface, 9 fixture+docs, 10 validate+close)
  are unaffected by this phase's changes; phase 8's `smelt explain`
  rendering could reasonably surface a succession model's probe records
  now that they exist, but that's phase 8's own scope to decide, not added
  here.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both
  feature sets, full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-runtime --test succession_probes --test
  succession_frontiers --test technique_lowering --quiet` — 3+5+37 passed.
- `cargo test -p smelt-runtime --test statement_parity --test
  execute_parity --quiet` — 4+41 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --quiet` — 101
  passed (full seeded sample).
- `cargo test -p smelt-maintenance-testkit --quiet` — 66 passed.
- `bash .claude/scripts/large-file-check.sh` — OK, no baseline regression.
