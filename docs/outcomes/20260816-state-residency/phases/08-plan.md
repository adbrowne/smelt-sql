# Phase 8 plan — state-deletion conformance leg

## Objective

Prove the residency rule end to end: deleting `.smelt/` mid-sequence (and re-running from a
fresh clone of the project directory) never breaks the equivalence invariant, for keyed additive
folds *and* idempotent-graded region-recompute models. Advances success criterion 5, and is the
end-to-end evidence for criterion 2 (the ledger and the frontier record are engine-resident, so
nothing correctness-class rides on `.smelt/`).

## Spec delta

None — this phase is a test-only leg over already-shipped behaviour. The Known Divergences
sweep across `state.md` / `run_state.md` / `incremental_models.md` is row 9. **Exception:** if a
step exposes a genuine residency defect (some correctness decision still riding on `.smelt/`),
fixing it is inside this phase — that is exactly what criterion 5 exists to catch — and the fix
carries its own spec sentence if user-visible.

## Tests

Red-green, all in a new `crates/smelt-cli/tests/maintenance_conformance/state_deletion.rs`
(registered in `main.rs`) unless noted.

1. `drop_state_dir_step_actually_removes_the_directory` — anti-vacuity: after a `RunWindow`,
   `project_dir/.smelt` exists; after a `DropStateDir` step it does not. Without this the whole
   leg can pass while testing nothing.
2. `keyed_additive_fold_survives_state_dir_deletion` — the flagship. `KeyedRecipe` with
   `KeyedCombiner::Additive` (the `_smelt_ledger`-guarded fold): run window 1, drop `.smelt/`,
   **redeliver window 1**, assert S-restricted equivalence (no double-count) and that the
   `_smelt_ledger` rows observed before the drop are still present after it.
3. `state_dir_deletion_mid_schedule_preserves_equivalence` — generative: the append-only
   `AdditiveAgg` pool, deterministic-seeded, N cases; a `DropStateDir` step injected at a
   generated index of each generated schedule; equivalence asserted after every subsequent run
   step by the existing `STracker` oracle.
4. `fresh_clone_mid_schedule_preserves_equivalence` — generative, same pool/seeding: a
   `FreshClone` step mid-schedule (project files copied to a new directory *without* `.smelt/`,
   same warehouse `db_path`); equivalence holds for every subsequent run driven against the
   clone. Distinct from (3) because the project path itself changes — anything keyed on the old
   absolute path is caught here and nowhere else.
5. `region_recompute_frontier_survives_state_dir_deletion` — idempotent-graded region-recompute
   (the fused DuckDB `DeleteInsert` path phase 7 landed): the per-batch `_smelt_frontier` rows
   recorded before the drop are byte-identical after it, and a post-drop rerun of an already-
   recomputed region still upholds equivalence.
6. `crates/smelt-maintenance-testkit` unit test `fresh_clone_copies_models_but_not_state` — the
   clone helper carries `models/` + `smelt.yml` and leaves `.smelt/` behind.

## Tasks

1. Add `ConformanceStep::DropStateDir` and `ConformanceStep::FreshClone` to
   `crates/smelt-maintenance-testkit/src/schedule_gen.rs`, each with a doc comment naming
   `state.md`'s residency rule; exclude both from `is_permutable` (a residency step is
   order-dependent by construction) and handle them in the existing step match at
   `schedule_gen.rs:606`.
2. Add `link_c_harness::LinkCProject::fresh_clone(&self, dest: &Path) -> Result<LinkCProject>`
   (copy `models/` recursively + `smelt.yml`, never `.smelt/`, reuse `db_path`) and derive
   `Clone` on `LinkCProject`.
3. Handle both new steps in `maintenance_conformance/gate.rs::drive_and_assert`: hold a local
   `let mut project = project.clone();` so `FreshClone` can replace the handle mid-loop;
   `DropStateDir` does `std::fs::remove_dir_all(project.project_dir.join(".smelt"))`.
4. Handle both new steps in `maintenance_conformance_spark/gate_spark.rs`'s match with an
   explicit `anyhow::bail!("residency steps are DuckDB-only …")` naming the ledger-less-backend
   downgrade (phase 5) as the reason — the Spark twin has no ledger builder, so it must refuse
   rather than silently pass. Compile-checked, not run.
5. Give the keyed driver a residency hook without touching `KeyedRunWindow`'s construction
   sites: `drive_keyed_and_assert_with_state_ops(project, recipe, schedule, ops: &BTreeMap<usize,
   StateResidencyOp>)` applying the op *before* window `i`; `drive_keyed_and_assert` delegates
   with an empty map. `StateResidencyOp { DropStateDir, FreshClone }` lives in the testkit
   alongside the step variants (one definition, two schedule shapes).
6. Add small read helpers in `state_deletion.rs` for the engine tables (`SELECT * FROM
   _smelt_ledger` / `_smelt_frontier`, sorted) via `project.backend()` — used by tests 2 and 5.
7. Write tests 1–6 red first, then wire the generative injection (tests 3–4) reusing
   `case_count()`'s `SMELT_CONFORMANCE_CASES` override convention and a small default (4).
8. If any test goes red for a *product* reason, stop and fix the product (see Spec delta note),
   recording the finding in the phase summary.

## Verification

- `bash .claude/scripts/verify-phase.sh` (full) — with `DUCKDB_LIB_DIR` / `LD_LIBRARY_PATH`
  exported inline.
- `cargo test -p smelt-cli --test maintenance_conformance 2>&1 | tail -40` — the whole gate,
  including the new leg.
- `cargo test -p smelt-maintenance-testkit 2>&1 | tail -20`.
- `cargo check -p smelt-cli --features smelt-cli/spark --tests 2>&1 | tail -20` — the Spark twin
  still compiles against the widened enum.
- `cargo test -p smelt-runtime --test frontier_residency --test state_posture 2>&1 | tail -20`.
- Anti-vacuity confirmation: temporarily comment out the `remove_dir_all` in task 3 and confirm
  test 1 fails; restore.

## Commit message

`test(conformance): prove equivalence survives .smelt/ deletion and a fresh clone`
