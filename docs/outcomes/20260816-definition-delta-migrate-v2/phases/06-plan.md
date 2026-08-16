# Phase 6 — `smelt migrate` reachable mid-incremental-history + migrate-driven recovery leg

## Objective

Close the production gap the phase-5 summary found: a maintained model must record the
definition it was last maintained under on **every** execution arm, so `smelt migrate` stops
failing closed with `NoRecordedDefinition` mid-incremental-history. Then drive the migration
mechanism itself through the conformance harness — rewrite → `smelt migrate` → `--apply` →
assert against the new-definition oracle. Advances success criteria 1, 2 and 4.

## The gap, precisely

`execute.rs` saves the deployed schema (which carries `definition_sql`) in two places: the
full-refresh branch (every run) and a first-deployment baseline for `plan.incremental.is_some()`
at the bottom of the per-model unit. The cumulative arm (`return Ok(ModelOutcome::Completed(..))`
~line 2743) and the key-addressed arm (~line 2883) both return **before** that baseline block, so
models taking those arms never record a definition at all. The `!already_stored` guard is
correct and stays: a windowed run under a *changed* definition must never overwrite the recorded
one, or the pending delta would silently vanish before `smelt migrate` could see it.

## Spec delta (implement step makes these edits first)

`docs/specs/definition_deltas.md`:

1. §Detection — after the "smelt records, per model, the definition it last maintained the
   stored table under" sentence, add the normative recording rule: the definition is recorded on
   **first deployment** regardless of which maintenance route the model takes, and re-recorded
   only by a full refresh or by `smelt migrate --apply`; a windowed/incremental run under changed
   SQL never overwrites it, so a pending definition delta survives until it is migrated.
2. §Known Divergences — add a bullet: §Detection's "`smelt run` refuses to fold data deltas while
   a non-eclipsed definition delta is pending" is not implemented; a windowed run today folds
   data deltas under the new SQL while the old definition stays recorded. Tracked as out of scope
   in `docs/outcomes/20260816-definition-delta-migrate-v2/outcome.md`.

## Tests (red first)

Runtime/CLI recording (`crates/smelt-cli/tests/incremental/schema_evolution.rs` or a new
`crates/smelt-cli/tests/definition_recording.rs`):

- `windowed_run_records_deployed_definition` — plain incremental (delete+insert) model, one
  windowed run → snapshot `definition_sql` equals the model file's raw text.
- `cumulative_run_records_deployed_definition` — a recipe taking the cumulative arm (early return
  ~2743) records it too.
- `key_addressed_run_records_deployed_definition` — same for the key-addressed arm (~2883).
- `windowed_run_after_rewrite_keeps_the_old_recorded_definition` — the fail-closed rule: rewrite
  the model, run another window, snapshot still holds the pre-rewrite definition.

`crates/smelt-cli/tests/migrate.rs`:

- `migrate_is_reachable_after_incremental_build` — incremental fixture, windowed `smelt build`,
  then `smelt migrate <model>` after an added-column edit exits `3` with a printed plan (no
  `NoRecordedDefinition` message on stderr).

`crates/smelt-maintenance-testkit/src/schedule_gen.rs` unit tests:

- `migrate_apply_step_is_not_permutable` — a schedule containing `MigrateApply` is excluded by
  `is_permutable`.

`crates/smelt-cli/tests/maintenance_conformance/gate.rs` (pinned, not generated):

- `migrate_apply_recovers_equivalence_after_payload_column_add` — PassThrough/Filter recipe:
  windows → `RewriteModel { AddPayloadColumn }` → `MigrateApply` → equivalence against the
  new-definition oracle (`assert_equivalence_with_edit`) with **no** intervening full refresh.
- `migrate_refuses_skeleton_change_then_full_refresh_recovers` — aggregate recipe with
  `AddGroupingColumn`: `MigrateApply` observes the refusal (exit `3`, table untouched), and the
  following `FullRefreshRun` recovers equivalence.

## Tasks

1. Write the four recording tests red; confirm which arms fail (expect cumulative + keyed).
2. Extract the first-deployment baseline save in `execute.rs` into one helper
   (`record_first_deployment_definition`), and call it from the cumulative and key-addressed arms
   before their early returns as well as the existing fall-through site. Keep `!already_stored`
   and the best-effort `tracing::warn!` posture; no new `unwrap`/`expect`.
3. Update `crates/smelt-cli/tests/maintenance_conformance/registry.rs`: the
   `known_bug_incremental_path_skips_schema_snapshot` entry's grep predicate closes — remove the
   entry (and its `known_bug_still_reproduces` arm) rather than re-pointing it at a new count.
4. Add the `migrate.rs` reachability test with an incremental fixture (`smelt.yml` +
   `materialization: incremental` frontmatter, windowed `--start/--end` build).
5. Add `ConformanceStep::MigrateApply` to `schedule_gen.rs` (doc comment: operator-directed
   migration recovery; no accompanying run) and add it to `is_permutable`'s exclusion list. The
   generator does **not** emit it in this phase — see Decision log.
6. Handle `MigrateApply` in `drive_and_assert` (gate.rs): drop every `duckdb::Connection` first
   (the CLI subprocess takes the file lock), spawn `env!("CARGO_BIN_EXE_smelt") migrate <model>
   --project-dir <dir> --target dev`, assert exit `3` + a printed plan, then spawn the same with
   `--apply`; record the observed exit code so the pinned refusal test can assert it. No
   `assert_equivalence` inside the step — the pinned tests own the assertion.
7. Handle the new variant in `crates/smelt-cli/tests/maintenance_conformance_spark/gate_spark.rs`
   — fail loud (`panic!` naming the variant as not part of the Spark pool), never a silent skip.
8. Add the two pinned gate tests.
9. Apply the two spec edits.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test migrate --features duckdb --quiet`
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb --quiet 2>&1 | tail -20`
- `cargo test -p smelt-maintenance-testkit --quiet`
- `cargo check -p smelt-cli --tests --features spark 2>&1 | tail -10`

## Commit message

`feat(migrate): record the deployed definition on every maintenance arm and drive migrate-apply recovery in the conformance harness`
