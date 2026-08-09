# Phase 5 plan — refusal narrowing: the plan actually derives a repair cell

## Objective

Wire `repair::admit_per_group_recompute` into `derive_maintenance_plan` so a keyed fold over a
mutable/retraction source **derives a `PerGroupRecompute` cell** instead of refusing outright, and
refuses **by repair obligation name** when it cannot. Also close phase 4's flag: thread a real
slice-completeness premise into `resolve_cell_choice`'s `DiffPatch` arm. Advances success criteria
1 and 2 (and the admission half of 3). Runtime lowering + executed-vs-emitted parity is phase 6.

## Spec delta

`docs/specs/incremental_models.md` §Known Divergences → "The contract, plan, and graph layer",
bullet "The repair family and `diff_patch` are specified ahead of derivation and emission":
narrow it to the remaining truth — per-group recompute and affected-key discovery **are** derived
by the plan; what is still missing is runtime lowering (no admitted repair cell executes yet) and
the executed-vs-emitted parity leg. Keep the outcome link.

## Design decisions this phase makes

- **Hook site: `derive_new_data`'s key-grain posture leg.** The narrowing fires exactly where the
  faithful-fold *source-posture* condition fails (`fold over 'X' fails the faithful-fold
  source-posture condition …`) — the retraction case criterion 1 names. Repair only ever converts
  a **refusal** into a cell; it never replaces an already-admitted `ColumnScopedMerge` or fold
  cell, so no existing admitted plan changes shape.
- **The combiner-algebra leg is not narrowed by this phase** (a holistic combiner over an
  append-only source). It is not a success criterion; no phase row.
- **Fail-closed refusal is additive.** When repair admission fails, the existing
  `NoAdmissibleTechnique` refusal is still pushed *and* the repair refusal
  (`Refusal::RepairKeysNotDiscoverable` / `RepairSliceUnbounded`) is pushed alongside it, naming
  the failing obligation. Existing refusal-text assertions keep passing; `smelt explain` gains the
  obligation name.
- **`DeltaShape` is derived, not a new world fact.** A `MutationProfile::MutableSnapshot` delta is
  a whole-row snapshot diff, so the delta carries every column of that source the model reads:
  build `columns` from the source's referenced columns via the same walk-backed
  `analysis::fingerprint` leaf classifier `affected_keys` already uses (no new `.contains("` scan,
  walk rule intact), and `keyed = !facts.unique_key.is_empty()`.
- **`derive_repair_cell` takes the trigger** instead of hard-coding `Trigger::UpstreamMutation`,
  so the cell it builds carries the trigger actually being derived.
- **`diff_patch` delete-leg premise:** in `resolve_cell_choice`'s `DiffPatch` arm, a
  `Technique::PerGroupRecompute` recompute discharges slice completeness (the repair family's own
  key-temporal-locality premise, already proven at admission) → `DeleteLeg::Present`; every other
  recompute keeps `DeleteLeg::Omitted` with a stated reason.

## Tests

Red-green, new file `crates/smelt-logical/tests/repair_wiring.rs` unless noted.

1. `keyed_fold_over_mutable_source_derives_a_per_group_recompute_cell` — a `grain: key` model with
   a fold column over a `MutableSnapshot` clocked source yields a `Technique::PerGroupRecompute` /
   `Corner::ColumnMerge` cell where today the plan carries only the posture refusal.
2. `repair_cell_carries_the_affected_key_and_the_bounded_slice` — the derived cell's
   `row_identity` is `RowIdentity::Key(<grain cols>)` and its `scans` is the
   `project_source_link` clamp, not empty.
3. `undiscoverable_affected_keys_refuses_repair_keys_not_discoverable` — an unkeyed source (no
   `unique_key`) still refuses, and the plan carries `Refusal::RepairKeysNotDiscoverable` naming
   the source alongside the pre-existing posture refusal.
4. `unclocked_mutable_source_refuses_repair_slice_unbounded` — an unclocked source refuses with
   `Refusal::RepairSliceUnbounded`, never a widened whole-table repair.
5. `append_only_fold_still_derives_the_unchanged_fold_cell` — no repair cell, no repair refusal.
6. `delta_shape_for_a_mutable_source_carries_its_referenced_columns` — unit over the new
   `DeltaShape` constructor.
7. `diff_patch_over_a_per_group_recompute_admits_the_delete_leg` (in `choice.rs` unit tests) —
   `ChosenTechnique::DiffPatch { delete_leg: DeleteLeg::Present, .. }`.
8. `diff_patch_over_a_delete_insert_recompute_still_omits_the_delete_leg` — the placeholder path
   stays explicit and reasoned, not silently widened.

## Tasks

1. Add `repair::delta_shape_for_source(sql, facts) -> DeltaShape` reusing the fingerprint leaf
   classifier; unit-test it (test 6).
2. Change `derive_repair_cell` to take the `Trigger` (update phase 3's callers/tests).
3. In `derive_new_data`'s key-grain posture-failure branch: attempt
   `admit_per_group_recompute`; on `Ok` push `derive_repair_cell(...)` for each eligible column
   group and skip the posture refusal's `return`-only behaviour; on `Err` push the mapped
   `Refusal::Repair*` alongside the existing `NoAdmissibleTechnique` (tests 1–5).
4. Map `RepairRefusal::{KeysNotDiscoverable, SliceUnbounded}` → `Refusal::{RepairKeysNotDiscoverable,
   RepairSliceUnbounded}` (add the variants' construction site; the enum already exists).
5. Thread the completeness premise through `resolve_cell_choice`'s `DiffPatch` arm; replace the
   placeholder `why` (tests 7–8).
6. Narrow the §Known Divergences bullet per the spec delta.
7. Check the fallout of newly-derived cells on `smelt-cli` explain goldens and
   `maintenance_conformance` pins; update goldens only where the new cell is the intended change.

## Verification

- `cargo test -p smelt-logical --test repair_wiring`
- `cargo test -p smelt-logical --test repair_cell --test diff_patch --test walk_coverage`
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering`
- `cargo test -p smelt-cli --test maintenance_conformance --test explain --test explain_model`
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`feat(incremental): derive a per-group repair cell where retraction refused`
