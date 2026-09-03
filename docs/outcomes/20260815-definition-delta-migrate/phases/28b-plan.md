# Phase 28b plan — pin the merged-group region-recompute rule

## Objective

Make the spec's merged-group rule true in the derivation and pin it: a column group whose
sensitivity spans two or more mutation-sensitive inputs is repaired by **region recompute**,
never a column-scoped merge. Advances success criterion 18 (the group-merge-provenance
decision recorded *and* honoured) by closing `incremental_models.md` §Known Divergences'
"merged-group region-recompute rule is unverified in the implementation" bullet.

**Planning-time audit (already done, do not redo):** `derive_mutation`
(`crates/smelt-logical/src/maintenance/derive.rs:1496-1618`) is called once per
mutation-capable input and picks `(Corner, Technique)` from `membership_sensitive` alone
(line ~1599). A group carrying two mutable inputs in its `mutation_sensitivity` set therefore
gets **two** independent `ColumnScopedMerge` cells today — exactly the shape the spec forbids.
The rule is violated, not merely unchecked: this is a code-change phase.

## Spec delta

No behavioural surface change — §"The plan matrix" already states the rule (lines ~891-896).
The only spec edit is the **removal** of the `incremental_models.md` §Known Divergences bullet
"**The merged-group region-recompute rule is unverified in the implementation**"
(lines ~2100-2104), plus a `last_reviewed` bump. Remove it in the same commit as the fix.

## Tests

New file `crates/smelt-logical/tests/maintenance_merged_group.rs` (red first):

1. `merged_group_takes_region_recompute` — a model whose one column group is value-sensitive to
   two clocked `mutable_snapshot` sources yields, for each source's `UpstreamMutation` trigger,
   a cell with `Corner::RecomputeRegion` / `Technique::DeleteInsert`; **no** `ColumnScopedMerge`
   cell exists for that group.
2. `single_mutable_input_group_keeps_the_column_merge` — the control: the same shape with only
   one mutable input still admits `Corner::ColumnMerge` / `Technique::ColumnScopedMerge`, so the
   guard is scoped, not a blanket downgrade.
3. `merged_group_rule_counts_only_mutation_capable_inputs` — a group sensitive to one mutable
   source plus one append-only source is *not* merged for this rule's purpose and keeps the
   column-scoped merge.
4. `membership_sensitivity_still_forces_recompute_for_a_single_input` — regression guard that
   the existing membership branch is unchanged by the new guard's placement.

End-to-end fixture pin (one test, nearest existing harness — prefer extending
`crates/smelt-runtime/tests/tracer_maintenance.rs`, else `crates/smelt-logical/tests/
maintenance_coverage_matrix.rs`):

5. `merged_group_fixture_plans_region_recompute` — a staged two-mutable-dimension model driven
   through the real plan-derivation path (not a hand-built `ModelInputs`) reports the
   region-recompute technique for the merged group in its plan/tracer output.

## Tasks

1. Write tests 1-4 red against today's derivation; confirm 1 fails with `ColumnScopedMerge`.
2. In `derive_mutation`, before the `(corner, technique)` choice, compute the group's
   **mutation-capable input count**: distinct names in
   `group.mutation_sensitivity ∪ group.membership_sensitivity` that the plan actually derives an
   `UpstreamMutation` trigger for (cross-check against `derive_triggers` so model edges and
   sources are counted on the same rule as the triggers themselves — record the verdict in a
   doc comment). Count ≥ 2 ⇒ `(Corner::RecomputeRegion, Technique::DeleteInsert)`.
3. Document the guard in a doc comment citing `incremental_models.md` §"The plan matrix" and
   the "conservative, always-correct default" rationale, in the style of the adjacent
   membership-sensitivity comment.
4. Run the full workspace suite and triage every changed expectation: any existing test that
   asserted `ColumnScopedMerge` for a two-mutable-input group was asserting the bug — update it
   and note the change in the summary. A test that trips for another reason is a real
   regression; do not paper over it.
5. Add the end-to-end fixture (test 5).
6. Delete the §Known Divergences bullet in `docs/specs/incremental_models.md`; bump
   `last_reviewed` to 2026-09-03.
7. Write `phases/28b-summary.md`; flip the row to `done` in `outcome.md`.

## Verification

- `cargo test -p smelt-logical --test maintenance_merged_group`
- `cargo test -p smelt-logical --test maintenance_coverage_matrix --test maintenance_choice
  --test maintenance_plan_admission --test maintenance_tracer`
- `cargo test -p smelt-runtime --test statement_parity --test tracer_maintenance`
- `cargo test -p smelt-cli --test maintenance_conformance` (the equivalence oracle — a
  merged group newly taking delete+insert must still match full refresh)
- `bash .claude/scripts/verify-phase.sh`
- `rg -n "merged-group region-recompute rule is unverified" docs/specs` → no hits

## Commit message

`fix(maintenance): repair a merged column group by region recompute, never a column-scoped merge`
