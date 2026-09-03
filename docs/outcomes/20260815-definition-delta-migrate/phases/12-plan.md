# Phase 12 — Per-cell frontier addressing: schedule per-cell `deferral`; lower `diff_patch` over the region `DeleteInsert` default

## Objective

Close success criterion 11's two divergences. (a) `contract.cells[].deferral` stops being
declaration-only: a per-cell maintained frontier is recorded in the ledger, and a cell whose
measured lag is within its own `D` has its trigger's work skipped and recorded, instead of the
model-level-only decision that exists today. (b) A `write: diff_patch` pin over a region
`DeleteInsert` cell gets a real runtime lowering over the region slice, so the pin is enforced
rather than silently ignored on the path that reaches it.

## Spec delta (made first)

- `docs/specs/incremental_models.md` §"The contract lattice" (deferral paragraph, ~line 818):
  state that a skip is decided **per cell** — the cell's own maintained frontier (its recorded
  per-cell interval end) against its `on:` trigger's landed frontier — and that a model whose
  other cells are outside their windows still runs them; the per-cell skip is recorded on the
  manifest against the cell, the existing model-level `skipped_deferral` (and its
  `skipped_deferral_upstream` propagation) is the case where every declaring cell skips.
- `docs/specs/incremental_models.md` §"`diff_patch` — compute, diff, write only the difference":
  state the region case's slice predicate and delete-leg verdict — a region recompute's write
  window clamp *is* its slice-completeness argument, so the region lowering carries
  `DeleteLeg::Complete` over the clamped window (the candidate is complete over exactly that
  window and nothing else).
- `docs/specs/run_state.md` §"Run manifest": document the new per-cell `deferred_cells` field.
- Remove both §Known Divergences bullets ("Per-cell `deferral` is not yet scheduled",
  "`diff_patch` over the region `DeleteInsert` default has no runtime lowering").

## Tests (red-green, in order)

`crates/smelt-logical/tests/contract_deferral_cells.rs` (new)
1. `cell_address_is_stable_for_group_and_trigger` — the pure cell-address function round-trips
   group+trigger and is order-insensitive within `columns`.
2. `cell_license_skips_only_within_its_own_d` — a cell with `lag <= d` licenses `Skip`; `lag > d`
   and `lag <= 0` do not; an unresolved (first-run) maintained frontier never skips.

`crates/smelt-state/tests/` (extend the intervals suite)
3. `cell_frontiers_default_when_absent_from_an_old_ledger` — a ledger JSON written without the
   per-cell map deserialises with an empty map (no ledger migration required).
4. `recording_a_cell_frontier_advances_only_that_cell` — recording one cell's end leaves sibling
   cells and the model-level covered intervals untouched.

`crates/smelt-runtime/tests/contract_deferral_schedule.rs` (extend)
5. `per_cell_decision_skips_one_cell_and_runs_its_sibling` — two `contract.cells[]` entries with
   different `D` against different `on:` sources; only the in-window one is licensed to skip.
6. `model_level_deferral_still_decides_when_no_cells_are_declared` — the existing model-level
   path is unchanged (regression guard).

`crates/smelt-runtime/tests/contract_deferral_skip_e2e.rs` (extend, real DuckDB)
7. `per_cell_skip_is_recorded_on_the_manifest_and_leaves_that_cells_frontier_unmoved` — the run
   completes, the manifest names the deferred cell, the skipped cell's frontier does not advance
   while the sibling's does.

`crates/smelt-runtime/tests/repair_lowering.rs` (extend, real DuckDB)
8. `diff_patch_over_a_region_delete_insert_cell_writes_only_the_difference` — RED today (pin
   silently ignored; run records the default write): asserts the run records `diff_patch` and
   that unchanged rows in the region are not rewritten.
9. `region_diff_patch_delete_leg_removes_rows_absent_from_the_regions_candidate` — the delete leg
   fires within the clamped window and touches nothing outside it.

`crates/smelt-runtime/tests/statement_parity.rs` — extend the executed-vs-emitted family list so
the region `diff_patch` statements are covered by the standing parity gate.

## Tasks

1. Write the spec delta above (spec-first).
2. `smelt-logical`: add the single-owned pure cell-address function and the per-cell
   `RunLicense` resolution to `contract/deferral.rs`, reusing `measure_lag`/`within_deferral`/
   `run_license` (no second oracle).
3. `smelt-state`: add `cell_frontiers: HashMap<String, String>` to `ModelIntervals`
   (`#[serde(default)]`) with a record/read accessor pair; add `deferred_cells: Vec<String>`
   (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`) to `ModelRunRecord`.
4. `smelt-runtime::contract_probes`: add `deferral_cell_decisions(...)` returning per-cell
   licenses — maintained frontier from the per-cell map (absent ⇒ no skip), input frontier from
   `LandedDeltaStore` for that cell's `on:` source.
5. `smelt-runtime::execute`: consult the per-cell decisions where trigger cells are dispatched —
   a licensed cell's trigger work is suppressed and its name pushed to `deferred_cells`; on a
   successful run, advance each non-skipped participating cell's frontier to the run's covered
   end. Model-level path unchanged; when every declaring cell skips, the existing model-level
   `skipped_deferral` record and dependent propagation still apply.
6. `smelt-logical::maintenance::choice`: thread the region recompute's own completeness premise —
   `ChosenTechnique::DiffPatch { recompute: Technique::DeleteInsert }` carries
   `DeleteLeg::Complete` over the clamped write window (replace the "not yet threaded" comment
   and its `Omitted` reason).
7. `smelt-runtime::maintenance_driver`: replace the `MaintenanceDiffPatchUnroutable` bail for
   `recompute: Technique::DeleteInsert` with a real `RepairWrite::DiffPatch` route whose
   `slice_predicate` is the region window predicate; keep the bail for any other recompute
   technique (still fail-loud by name).
8. `resolve_live_membership_recompute_cell`: stop `continue`-ing past a `DiffPatch` choice — route
   it to the new lowering so the pin is enforced on the one path that reaches it.
9. Run the gates; write `phases/12-summary.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test contract_deferral_cells --test diff_patch`
- `cargo test -p smelt-runtime --test contract_deferral_schedule --test contract_deferral_skip_e2e --test contract_deferral_probe --test repair_lowering --test statement_parity --test execute_parity`
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb`
- `cargo test -p smelt-db --test contract_deferral_diagnostics`

## Commit message

`feat(contract): schedule per-cell deferral over a per-cell frontier record and lower diff_patch over the region DeleteInsert default`
