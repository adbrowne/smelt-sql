# Phase 6 plan — live dispatch of the source append-only posture probe

## Objective

Give the last `built (unwired)` registry row a live dispatch: a run that consumes a
source declaring `mutation_profile.kind: append_only` re-checks that source's recorded
per-partition counts and closed-partition fingerprints before it writes, failing loud with
`SourceMutationProfileViolated`. Advances criteria 2 (all four probes wired), 4 (named
diagnostic + remedy) and 5 (cadence-governed), and is the last prerequisite for criterion 6's
fact-violation recipes.

## Design decisions taken here

- **Baseline persistence.** A new `smelt-state` store (`source_postures.json`), same
  load-modify-save-under-`state_io_lock` pattern as `landed_deltas.json`. Entry:
  `{ source, partition_value, recorded_count, recorded_fingerprint }`.
- **Frontier gate (the phase title's "frontier-fingerprint re-check").** A partition that is
  still open legitimately receives appends, which change its whole-partition fingerprint. So
  the *count* leg applies to every recorded partition; the *fingerprint* leg applies only to
  partitions strictly below the recorded maximum partition value. The gate is caller data, not
  emitter policy: `AppendOnlyBaselinePartition` gains `check_fingerprint: bool`, rendered as a
  fourth `VALUES` column, and the emitter's predicate becomes
  `current_count < recorded_count OR (check_fingerprint AND fingerprint IS DISTINCT FROM …)`.
- **Cadence skip does not re-record.** A policy-skipped run leaves the baseline untouched, so
  the next dispatched run still compares against the last *verified* point. Recording happens
  only on a run that actually dispatched and held.
- **Declared `mutation_profile.lateness` is not consulted** — a source that appends into an
  already-closed partition beyond the open one can still fire. Recorded as a known divergence
  pointing at the frozen-horizon work (outcome §Out of scope), not silently absorbed.

## Spec delta (implement step makes these edits first)

- `docs/specs/model_properties.md` §"Probe obligation": the `mutation_profile.kind:
  append_only` row's Status `built (unwired)` → `built`; its scope cell names the recorded
  baseline store and the closed-partition fingerprint gate. §Known Divergences: drop the
  "one of seven rows has an emitter but no live dispatch" paragraph, replace with the lateness
  limitation above (source `unique_key`/`delta_identity` stays the one row with no emitter).
- `docs/specs/sources.md` §Known Divergences (the "Declared profiles license almost nothing
  yet" bullet): `SourceMutationProfileViolated` now exists and dispatches; keep
  `SourceWatermarkViolated`/`SourceUniqueKeyViolated`/`SourceRetentionExceeded` listed as
  unbuilt. §Semantics 4's wording stays — the built behaviour must match it.

## Tests (red first)

- `smelt-logical/tests/…` (extend the phase-2 append-only emitter tests, real DuckDB):
  - `append_into_open_partition_does_not_violate` — `check_fingerprint: false` on the max
    partition; appended rows hold.
  - `in_place_update_of_closed_partition_violates` — equal count, changed content, fires.
  - `count_decrease_violates_even_when_fingerprint_unchecked` — count leg is ungated.
- `smelt-state/tests/source_postures.rs`:
  - `store_round_trips_and_replaces_a_sources_partitions`
  - `closed_baseline_marks_every_partition_but_the_max_as_fingerprint_checked`
- `smelt-runtime/tests/source_probes.rs` (real DuckDB):
  - `first_run_has_no_baseline_so_builds_no_probe_and_records_one`
  - `second_run_over_mutated_source_fires_with_the_registry_code_and_remedy`
  - `second_run_over_appended_source_holds_and_refreshes_the_baseline`
  - `cadence_off_skips_dispatch_and_leaves_the_baseline_untouched`
- `smelt-runtime/tests/statement_parity.rs`:
  - `append_only_posture_probe_and_baseline_snapshot_come_from_the_emitters` — executed SQL
    byte-identical to direct `emit_append_only_posture_probe` /
    `emit_append_only_baseline_snapshot` calls.
- `smelt-cli/tests/e2e/…` (extend `declared_fact_probe_firing.rs`):
  - `append_only_source_mutated_between_runs_fails_the_second_run` — model table unchanged.
  - `probes_cadence_off_lets_the_mutating_run_write`.
- `smelt-logical/tests/probe_obligation.rs` — the append-only row's Status is `built`; no
  `built (unwired)` rows remain.

## Tasks

1. Spec edits above (spec-first).
2. `smelt-logical/src/maintenance/emit.rs`: add `check_fingerprint` to
   `AppendOnlyBaselinePartition` and gate the fingerprint leg; extract the shared
   per-partition current-state `SELECT` (partition_value / current_count /
   current_fingerprint) and add `emit_append_only_baseline_snapshot(source_table,
   partition_column, digest_columns, dialect)` returning it, so recorded and compared
   fingerprints are the same construction by definition.
3. `smelt-state/src/source_postures.rs` + `FileStore::load_source_postures` /
   `save_source_postures`; `closed_baseline(source)` returns
   `Vec<AppendOnlyBaselinePartition>`-shaped rows with the frontier gate applied (state must
   not depend on `smelt-logical` — return its own row type, converted in `smelt-runtime`).
4. `smelt-runtime/src/source_probes.rs` (new, exported from `lib.rs`):
   `append_only_posture_probes(...)` (pure builder over the model's consumed `SourceInfo`s —
   `mutation_profile.kind == AppendOnly` + `timeseries.partition_column` + non-empty declared
   `columns:` as digest columns + non-empty recorded baseline; physical table via
   `db_name_for_target`) and `dispatch_and_record_append_only_postures(...)` (dispatch each
   through `probes::dispatch_probe` with `ProbeContext { probe_code:
   "SourceMutationProfileViolated", fact: "mutation_profile.kind: append_only", … }`, fail loud
   on `Violated`, then execute the baseline snapshot and return the refreshed rows).
5. Wire both pre-write sites in `execute.rs` that already call
   `dispatch_declared_model_probes` (full-refresh arm and incremental batch loop), reusing
   `probe_policy_for_model` and the source list `build_maint_source_facts` already resolves;
   persist the refreshed baseline under `state_io_lock` (same critical-section shape as the
   landed-delta recording).
6. Flip the registry row and run the gates.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test source_probes --test model_probes --test probe_dispatch --test statement_parity --test execute_parity`
- `cargo test -p smelt-logical --test probe_obligation --test walk_coverage`
- `cargo test -p smelt-state --test source_postures`
- `cargo test -p smelt-cli --test e2e --test maintenance_conformance --test example_diagnostics`

## Commit message

`feat(probes): dispatch the source append-only posture probe against a recorded per-partition baseline`
