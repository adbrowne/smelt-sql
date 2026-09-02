# Phase 26b — per-arm classification for set-operation models

## Objective

Close the `INTERSECT`/`EXCEPT` clause of success criterion 16: a model whose outermost query is a
set operation stops collapsing to one degenerate whole-model column group and instead gets a real
per-arm verdict — value provenance combined positionally across the *value-contributing* arms, and
the cardinality-deciding arms folded into membership sensitivity. Fail-closed fallbacks remain for
the chain shapes this classification cannot pose.

## Spec delta (first)

- `docs/specs/model_properties.md` §"Per-column mutation-sensitivity / column provenance": add an
  "Across set-operation arms" paragraph stating the combination rule below, that output column
  names come from the first arm, and that a shape outside the rule collapses whole-model.
  §Known Divergences: narrow the `INTERSECT`/`EXCEPT` bullet to **filter distribution only** (still
  true) and drop its mutation-sensitivity cross-ref; add the residual fallback list (mixed-op
  chain, nested compound arm, arity mismatch) as a stated limitation, not a silent one.
- `docs/specs/incremental_models.md` §Known Divergences: delete the "`INTERSECT`/`EXCEPT` are
  unclassified set operations" bullet (line ~2073).

The combination rule (chain of one repeated op over arms `a0..an`, output positions from `a0`):

| chain op | value arms (per position) | membership contribution |
|---|---|---|
| `UNION ALL` | every arm | each arm's own membership set |
| `UNION` (distinct) | every arm | + every arm's *referenced* source names (dedup couples arms) |
| `INTERSECT[ ALL]` | every arm | + every arm's referenced source names |
| `EXCEPT[ ALL]` | first arm only | + every arm's referenced source names (the subtrahend decides existence) |

"Referenced source names" is the pre-`contributes` set — an append-only insert into an `EXCEPT`
right arm still deletes an output row, so the mutation profile does not filter the membership leg.

## Tests (red-green)

`crates/smelt-logical/tests/maintenance_grouping.rs` (extend):
1. `union_all_arms_combine_provenance_per_position` — two arms reading different sources yield
   more than one group, no degenerate entry (today: one whole-model group).
2. `except_right_arm_is_membership_only` — the right arm's source appears in
   `membership_sensitivity` and in no group's `mutation_sensitivity`.
3. `intersect_arms_contribute_to_value_and_membership` — every arm's source lands in both legs.
4. `union_distinct_couples_arms_into_membership` — dedup adds arm sources to membership that
   `UNION ALL` over the same arms does not.
5. `mixed_op_chain_falls_back_to_whole_model` — `a UNION ALL b EXCEPT c` degenerates, reason names
   the mixed chain.
6. `nested_compound_arm_falls_back_to_whole_model` — a parenthesised compound arm degenerates.
7. `arity_mismatch_across_arms_falls_back_to_whole_model`.
8. `unresolvable_reference_in_one_arm_collapses_whole_model` — fail-closed; the arm's own
   per-column reason is preserved in `degenerate`.
9. `single_select_grouping_is_unchanged` — regression anchor for the refactor (a plain SELECT
   model's groups and degenerate list are byte-identical to today's).

`crates/smelt-logical/tests/maintenance_plan_admission.rs` (extend):
10. `setop_model_admits_a_narrower_cell_than_whole_model_recompute` — the derived plan for a
    `UNION ALL` model carries per-group cells instead of the blanket whole-model group.

## Tasks

1. Write the spec delta above.
2. In `crates/smelt-logical/src/maintenance/grouping.rs`, extract the post-`QueryTree` body of
   `derive_column_groups` into `classify_arm(tree, select, sources, source_by_name,
   skip_positions) -> ArmClassification` returning ordered `Vec<(alias, BTreeSet<String>)>` (all
   select items, in position order), the arm's `referenced_sources`, its `membership_sensitivity`,
   and its `degenerate` reasons. `skip_positions` reproduces today's skeleton-column skip so the
   single-arm path stays behaviour-identical (a skeleton column's unresolvable reference must still
   not collapse the model).
3. Add `setop_arm_trees(&QueryTree) -> Option<Vec<QueryTree>>` in the same module: for a
   `QueryNode::SetOp` root, one tree per branch whose root is that branch's `SelectNode` with the
   compound's hoisted `ctes` prepended; `None` when any branch is not a `Select`. `QueryNode` is
   `Clone`, so this is a pure re-rooting — no walk.rs change.
4. Replace the `matches!(tree.root, QueryNode::SetOp(_))` early return with the arm path: reject
   (whole-model degenerate, each with its own reason) a chain whose `ops` are not all equal, a
   `None` from task 3, or an arity mismatch; otherwise classify every arm and combine per the
   table above. Any arm that degenerates collapses the whole model, carrying that arm's reasons.
5. Output column names/skeleton filtering come from arm 0; positions beyond arm 0 are matched by
   index, never by name.
6. Rewrite the module doc's set-operation sentence (it documents the collapse this phase removes)
   and record the fallback list there.

Out of this phase: `analysis/affected_keys.rs`'s own set-operation refusal (key discovery, a safe
`NotDiscoverable`) — no success criterion names it, and it is a different proof.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test maintenance_grouping --test maintenance_plan_admission --test maintenance_plan_refusals --test maintenance_coverage_matrix`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-db --test maintenance_diagnostics`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test --workspace` (phase 25's summary: a sensitivity-taxonomy change broke tests outside
  the phase's own file list — sweep before declaring green)

## Commit message

`feat(maintenance): classify mutation-sensitivity per set-operation arm`
