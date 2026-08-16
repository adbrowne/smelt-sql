# Phase 8 plan — conformance recipes for the repair + diff-patch families

## Objective

Give the standing generative gate (`cargo test -p smelt-cli --test maintenance_conformance`)
typed recipe coverage of the two families this outcome added: a keyed non-invertible fold over a
*mutable, clocked* source driven through retraction/mutation steps (per-group repair), and the
same recipe pinned `write: diff_patch` driven through a reconcile/re-run step. Each case is staged
as recipe **data** in `smelt-maintenance-testkit` (never a hand-written SQL string in the test),
driven end-to-end through the real `execute_project` pipeline, and asserted multiset-equal to a
full-refresh oracle after every run step. Advances success criteria 4 (conformance recipes), and
closes the executed-vs-oracle half of 1/2/3.

## Spec delta

None. This phase adds test coverage only — no user-visible behaviour changes. (If a case turns up
a real divergence, it becomes a registry entry per `registry.rs`'s `KnownBug` family plus a
Known Divergences line, not a silent skip.)

## Tests

Red-green list, all in a new `crates/smelt-cli/tests/maintenance_conformance/repair.rs`
(`gate.rs` is already ~5.9k lines; its helpers are `pub` and are reused, not copied):

1. `repair_recipe_admits_per_group_recompute` — a staged `RepairRecipe` classifies `Admitted` with
   at least one `Technique::PerGroupRecompute` cell (criterion 1), and the cell's key + scan clamp
   match the recipe's declared `unique_key` / band (criterion 2).
2. `repair_pool_upholds_equivalence_under_retraction` — for each combiner in the small deterministic
   repair pool, drive a schedule of insert → update-in-place → delete steps; after **every** run
   step the model table is multiset-equal to the full-refresh oracle over the current source state.
3. `repair_run_actually_executes_the_repair_family` — the captured statements for a retraction step
   contain the repair family's affected-key `EXISTS` slice and its targeted delete+insert, i.e.
   equivalence is not passing because the run silently fell back to full refresh.
4. `diff_patch_pinned_repair_upholds_equivalence_under_reconcile` — the same recipe pinned
   `write: diff_patch`: a mutation step followed by a *re-run of the same window with no source
   change* keeps equivalence, and the second run writes nothing (empty diff) — reconciliation and
   idempotent re-run (criteria 3/4).
5. `diff_patch_run_is_labelled_and_emitted` — the pinned run's captured statements are the
   `emit_diff_patch` statements (delete leg present, restricted to the affected-key slice), not the
   default targeted delete+insert.

## Tasks

1. Add `RepairRecipe` to `smelt-maintenance-testkit::recipe`: model name, `KeyedCombiner`
   (non-invertible members only — `Idempotent`/`OrderMonotone` — as `arb_repair_combiner`), band
   width, and a `RepairWriteMode { TargetedDeleteInsert, DiffPatch }`.
2. Add its rendering to `render.rs`: the clocked `mutation_profile: mutable_snapshot` source with a
   declared `unique_key` (the shape `crates/smelt-runtime/tests/repair_lowering.rs`'s fixture uses),
   the keyed model file with the Form B band, and the `maintenance: cells: [... write: diff_patch]`
   block for the pinned variant. Match the existing `render_keyed_model_file` idiom.
3. Add a `RepairSchedule` (insert / update / delete / re-run steps) plus its deterministic
   constructor — no proptest draw needed beyond a fixed combiner sweep; keep the case count small
   and honour the `SMELT_CONFORMANCE_CASES` convention if a sweep is introduced.
4. New `repair.rs` module in `crates/smelt-cli/tests/maintenance_conformance/` (+ `mod repair;` in
   `main.rs`): `stage_repair_recipe`, `classify_repair`, `drive_repair_and_assert` built on
   `LinkCProject`, `SqlCapturingReporter`, `multiset_equal_via_backend` and gate.rs's existing
   `assert_*` helpers.
5. Write tests 1–5 red first, then wire until green.
6. If any case diverges from the oracle, stop and record it: registry entry + Known Divergences
   line naming the call site, and flag it in the phase summary for phase 9 / a follow-up outcome.
   Do not weaken the oracle.
7. Update `pinned.rs`'s retired-probe/coverage doc table only if a mapping actually changes;
   otherwise leave it alone.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test maintenance_conformance 2>&1 | tail -40`
- `cargo test -p smelt-runtime --test statement_parity --test repair_lowering 2>&1 | tail -20`
  (the new testkit rendering must not perturb the existing repair fixtures)

## Commit message

`test(incremental): conformance recipes for per-group repair and diff-patch`
