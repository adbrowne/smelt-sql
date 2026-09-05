# Phase 8 plan — `state.mode` honoured in `execute_project`

## Objective

Make the project's `state.mode` posture decide what a run writes under `.smelt/`: `stateless`
(the default) writes nothing at all — no directory, no lock, no meta — and each higher posture
writes exactly the structures `state.md` §"`state.mode` and what each posture provides" lists.
Capabilities that consume an excluded observability structure degrade or refuse by name rather
than pretending the state was empty. Advances success criterion 2.

## Spec delta (made first, by the implement step)

`docs/specs/state.md` — two structures smelt already writes are absent from the normative
tables, which this phase's per-posture write set must enumerate exhaustively:

1. §"The state-structure inventory" — add two `observability` rows: **source-mutation baselines**
   (`.smelt/targets/<t>/source_mutations.json`, owner `sources.md`) and **migration approvals**
   (`.smelt/targets/<t>/migration_approvals.json`, owner `definition_deltas.md`).
2. §"`state.mode` and what each posture provides" — the `intervals` row's list gains both names
   (they are ordinary observability state; `environments` inherits them).
3. §Semantics "The optionality rule" — one clause naming `--resume` under `stateless` as a
   refuse-by-name case (it has no manifest to resume from *by posture*, not by accident).

No change to `run_state.md` §"Stateless writes nothing" — it already states the rule this phase
implements.

## Tests

Red-green, in this order.

- `smelt-state/src/file_store.rs::written_artifacts_match_the_posture_table` — the pure
  `state_artifacts_written(mode)` set equals the spec's consequence table for all three postures
  (stateless = empty; intervals = everything but the snapshot store; environments = all).
- `smelt-state/src/file_store.rs::stateless_store_writes_nothing` — every `save_*`/`init`/`lock`
  on a `stateless` `FileStore` succeeds and leaves the project dir without a `.smelt/` entry.
- `smelt-state/src/file_store.rs::intervals_store_denies_snapshot_store` — `save_snapshot_store`
  under `intervals` writes no file; under `environments` it does.
- `smelt-state/src/file_store.rs::stateless_loads_return_defaults_over_stale_files` — a `.smelt/`
  left over from a higher posture is not read back under `stateless`.
- `smelt-cli/tests/state_posture.rs::stateless_run_creates_no_smelt_dir` — a real DuckDB run via
  `execute_project` with `state.mode: stateless` leaves no `.smelt/`; the models are still built.
- `smelt-cli/tests/state_posture.rs::intervals_run_writes_exactly_the_posture_set` — same project
  at `intervals`: assert the exact set of paths under `.smelt/` (manifest, report, intervals,
  meta; no `snapshots.json`).
- `smelt-cli/tests/state_posture.rs::environments_run_adds_the_snapshot_store` — `environments`
  is a superset of `intervals`.
- `smelt-cli/tests/state_posture.rs::resume_under_stateless_refuses_naming_the_posture` —
  `--resume` errors with a message naming `state.mode: stateless`, not the generic
  "no partially-failed run" text.
- `smelt-cli/tests/state_posture.rs::stateless_deferral_cell_folds_every_run` — a
  `contract.deferral` model under `stateless` never takes the skip (coarser, always-correct
  degradation) rather than skipping on absent frontier state.
- `smelt-cli/tests/state_posture.rs::history_under_stateless_names_the_posture` — `smelt history`
  on a stateless project reports the posture instead of an empty list.

## Tasks

1. Land the three `state.md` edits above.
2. In `smelt-state/src/file_store.rs`: add `StateArtifact` + the pure
   `state_artifacts_written(StateMode) -> &'static [StateArtifact]` table and a
   `FileStore::with_state_mode(project_dir, target, mode)` constructor storing the posture
   (`FileStore::new` keeps today's permissive posture — it is the read/tooling constructor).
3. Gate every `save_*`, `delete_schema`, `init`, `lock` and `check_and_upgrade_meta_locked` on
   `self.allows(artifact)`: denied → `Ok(())` no-op that touches no path. Gate the `load_*`
   readers the same way, returning the default store when the posture excludes the artifact.
4. `execute.rs`: build the store with `config.state.mode`; make the `.smelt/lock` acquisition
   conditional on the posture writing anything; make `--resume` under `stateless` refuse with a
   posture-naming error before it reaches the history load.
5. `smelt-cli` `history`/`status`: when the project is `stateless` and no `.smelt/` exists, say
   the posture is why rather than printing an empty result.
6. Update the fixtures whose Configs are built in code and which assert `.smelt/` artifacts
   (`smelt-cli/tests/{resume,run_report}.rs`, `tests/incremental/{intervals,schema_evolution}.rs`,
   `tests/maintenance_conformance/gate.rs`, `smelt-runtime/tests/{contract_deferral_skip_e2e,
   schema_migration_backfill_atomicity}.rs`, `smelt-maintenance-testkit`) to declare
   `state.mode: intervals`; any example project whose test asserts run history gets the same key
   in its `smelt.yml`.
7. Add a structural assertion (unit test or `rg` gate) that `execute.rs` constructs the store via
   `with_state_mode`, so a future call site cannot reintroduce the permissive posture in the run
   pipeline.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-state --lib`
- `cargo test -p smelt-cli --test state_posture --test resume --test run_report --test incremental`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-runtime --test execute_parity --test statement_parity`

## Commit message

`feat(state-residency): honour state.mode in execute_project — per-posture .smelt/ write set`
