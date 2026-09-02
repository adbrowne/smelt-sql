# Phase 19b — Mutation-happened discrimination

## Objective

An `UpstreamMutation` cell currently re-checks and (where live) re-applies on every run, whether
or not the source actually changed. Record a per-source content-fingerprint baseline and use it to
decide, before dispatch, whether the mutation genuinely happened: unchanged source → the cell is
recorded as a no-op and executes no statements; changed (or nothing recorded yet) → dispatch as
today, then re-record. Closes the last clause of success criterion 15's "dispatch distinguishes
'a mutation genuinely happened' from re-derivation" and removes `incremental_models.md`'s
"Plan-consumer gaps" Known Divergences bullet outright.

## Spec delta (spec-first — make this edit before the code)

`docs/specs/incremental_models.md`:
- §"Per-cell admission" (beside the "Which changed inputs get a mutation cell" paragraph phase 19
  landed): add a timeless paragraph **"When a mutation cell dispatches"** — a run compares the
  source's recorded content fingerprint (row count + order-independent row digest over the
  source's digest columns) against the source's current fingerprint. Equal → the cell is recorded
  as a no-op for this run and executes nothing; differing, digest-column-set drift, or no
  recorded baseline → the cell dispatches and the observed fingerprint is re-recorded. Recording
  happens only on a run that actually dispatched (same discipline as the append-only posture
  baseline), so a failed run cannot suppress the next run's cell.
- §Known Divergences: delete the "**Plan-consumer gaps**" bullet (lines ~1936–1940); it now names
  only this clause.

## Tests (red-green)

- `smelt-logical` (`crates/smelt-logical/tests/` or emit unit tests) —
  `emit_source_mutation_fingerprint_duckdb_shape`, `…_spark_shape`, `…_bigquery_shape`: the
  whole-source (unpartitioned) `COUNT(*)` + order-independent aggregate fingerprint SELECT, one
  per `MaintenanceDialect`, reusing `row_fingerprint_expr`.
- `smelt-state` — `source_mutation_store_round_trips`: save/load `SourceMutationStore` through
  `FileStore`; `get` on an unrecorded source is `None`.
- `smelt-runtime` unit (`mutation_probe.rs` `#[cfg(test)]`) — `dispatch_when_no_baseline`,
  `noop_when_count_and_fingerprint_unchanged`, `dispatch_when_fingerprint_changed`,
  `dispatch_when_count_changed`, `dispatch_when_digest_column_set_changed`.
- `crates/smelt-runtime/tests/statement_parity.rs` —
  `source_mutation_fingerprint_comes_from_the_emitter`: drive the runtime probe helper against the
  `RecordingBackend` and assert the executed SQL is byte-identical to
  `emit_source_mutation_fingerprint`'s output (statement-emission single owner).
- `crates/smelt-runtime/tests/technique_lowering.rs` —
  `column_scoped_merge_skipped_when_dimension_unmutated`: sibling of the existing
  `column_scoped_merge_matches_full_refresh_after_dimension_mutation`. Run once (records the
  baseline), re-run with the dimension untouched → no `MERGE` executed for the cell and the run
  result still equals the full-refresh oracle; then mutate the dimension and re-run → the cell
  dispatches again and equals the oracle.

## Tasks

1. Spec delta above (both edits), before any code.
2. `crates/smelt-logical/src/maintenance/emit.rs`: `emit_source_mutation_fingerprint(source_table,
   digest_columns, dialect) -> MaintenanceStatement` — whole-source variant of
   `emit_append_only_baseline_snapshot` (no `GROUP BY`, no partition column); panic on empty
   digest columns, same three per-dialect aggregate forms.
3. `crates/smelt-state/src/source_mutations.rs`: `SourceMutationBaseline { recorded_count: i64,
   recorded_fingerprint: String, digest_columns: Vec<String> }` + `SourceMutationStore` with
   `record`/`get`; `FileStore::{load,save}_source_mutations` (`source_mutations.json`), registered
   wherever the other stores are listed/initialised.
4. `crates/smelt-runtime/src/mutation_probe.rs`: the pure verdict
   `decide_mutation_dispatch(baseline: Option<&SourceMutationBaseline>, observed: &ObservedSourceFingerprint)
   -> MutationVerdict::{Dispatch, NoOp}`, plus the async helper that emits the statement, reads the
   single row, and returns the observed fingerprint. Digest-column-set mismatch ⇒ `Dispatch`
   (a changed digest set makes the recorded value incomparable, never a silent skip).
5. `crates/smelt-runtime/src/execute.rs`: route **every** live `UpstreamMutation` dispatch site
   (the keyed-run site at ~L2508 and the incremental-branch sites at ~L2885/~L3029/~L3543) through
   one helper that runs the probe, skips on `NoOp`, and re-records only after a dispatch
   succeeded. Surface the no-op in the run report the same way other skipped maintenance work is
   reported, so `smelt explain`/reporter output names it rather than staying silent.
6. Fix the now-stale comment at `execute.rs` ~L2058 (phase-19 summary: it still claims
   `UpstreamMutation` is derived only for an unclocked source).
7. Re-check no example workspace regresses (`example_diagnostics`, `example_workspaces`).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test maintenance_plan_admission --quiet`
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering --test execute_parity --quiet`
- `cargo test -p smelt-cli --features duckdb --test maintenance_conformance --quiet`
- `cargo test -p smelt-lsp --test example_workspaces --quiet`

## Commit message

`feat(maintenance): discriminate a genuine upstream mutation from re-derivation via a recorded per-source fingerprint baseline`
