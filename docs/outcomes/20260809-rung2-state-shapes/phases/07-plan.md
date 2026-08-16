# Phase 7 plan — `AVG`/`STDDEV_*`/`VAR_*` decomposed folds at keyed grain

## Objective

Widen keyed admission to the decomposed-fold family so `AVG(x)`, `STDDEV_*`/`VAR_*` fold
incrementally on hidden `(sum, count)` / `(n, sx, sxx)` state instead of refusing (success
criterion 3), and make every state-blind consumer state-aware in the same pass: the ledger
grade (the first *additive* hidden state), the driver's defence-in-depth `refuse()`, the plan
layer's faithful-fold algebra leg, and `smelt explain`'s keyed-fold preview folds.

## Spec delta (spec-first)

- `docs/specs/incremental_models.md` §Known Divergences — delete the sentence "`AVG`/`STDDEV_*`/
  `VAR_*` folding at keyed grain still refuses rather than consume the mechanism" from the
  "Ladder rung 2 is specified but only partially wired" bullet, and drop the bullet entirely if
  nothing residual remains (the normative catalogue/matrix rows already describe this family as
  admitted; nothing else in the spec body changes).
- `docs-site/docs/reference/cumulative-aggregate.md` — remove `AVG` from the "**Out of v1**" list
  and add the decomposed-fold family (`AVG`, `STDDEV_*`, `VAR_*`) to the admitted-combiner
  material, noting the state is hidden and the presented value is recomputed from it.

## Design decisions (settle before coding)

1. **The presented column's combiner.** A decomposed fold has no target/delta formula for its
   presented column — it is recomputed by `π(merged state)`. Add
   `CrossPartitionCombiner::Recomputed`; its `render` returns `target_col` unchanged, documented
   as unreachable because `expand_aggregator_column_folds` always takes the state branch for a
   state-bearing column and `refuse()` (task 4) rejects a `Recomputed` column carrying
   `state: None` before any statement is built. Rejected: making `render` fallible (ripples a
   `Result` through `merge_sql`'s trait signature) and replacing the
   `(cross_partition_combiner, state)` pair with a `ColumnFold` enum (right shape, ~40
   construction sites — a refactor that would swamp this phase; note it for a later outcome).
2. **The algebra widening lives in `faithful_fold`, not in a `derive.rs` waiver.** Unlike
   once-write's contextual `Coalesce` waiver, "AVG decomposes into monoid state" is a
   family-level algebraic fact. Widen condition (2) with a new pure predicate
   `decomposed_state::has_monoid_state_shape(function, distinct)`; `derive.rs` then needs no new
   exemption. `faithful_fold` has exactly two call sites, both in `derive.rs`.

## Tests (red first)

`crates/smelt-logical/tests/keyed_families.rs`
1. `avg_column_admits_on_decomposed_sum_count_state` — `AVG(amount) AS avg_amount` admits with
   `__sum`/`__count` state columns (both `Sum`) and combiner `Recomputed`.
2. `stddev_samp_column_admits_on_welford_state` — `(n, sx, sxx)` state, presented expr is the
   family's closed form.
3. `avg_distinct_refuses_as_holistic` — `AVG(DISTINCT amount)` → `KeyedUnknownCombiner`.
4. `avg_composite_expression_refuses` — `AVG(x) + 1` still refuses (direct-call check).
5. `avg_under_snapshot_reconcile_refuses` — `KeyedSnapshotSourceUnsupportedColumn` with family
   `decomposed fold`.
6. `median_still_refuses_as_unknown_combiner` — regression: a holistic aggregate with no encoded
   state shape stays refused.

`crates/smelt-logical/src/analysis/faithful_fold.rs` (unit)
7. `avg_passes_submultiset_fold_via_encoded_monoid_state` / `median_still_fails_submultiset_fold`.

`crates/smelt-logical/tests/emit_statements.rs`
8. `avg_keyed_merge_folds_state_and_recomputes_average` — the merge folds `__sum`/`__count`
   pairwise and sets the presented column to `(target.__sum + delta.__sum) / (target.__count +
   delta.__count)`.

`crates/smelt-runtime/src/cumulative.rs` (unit)
9. `avg_model_is_ledger_graded_additive` — state-aware `ledger_grade`.
10. `max_by_and_once_write_state_stay_idempotent` — regression: non-additive state keeps
    `Idempotent` (no ledger table appears for them).
11. `refuse_accepts_state_bearing_avg_column` — defence-in-depth admits it; a `Recomputed` column
    with `state: None` is refused with a named internal-invariant message.

`crates/smelt-db/tests/maintenance_fold_spec_companion.rs`
12. `avg_model_derives_fold_spec_and_keyed_fold_cell` — `derive_fold_spec` includes the `AVG`
    column and the derived plan carries a `Technique::KeyedFold` cell (no
    `NoAdmissibleTechnique` refusal); a wrong-arity/`DISTINCT` `AVG` refuses the derivation.

`crates/smelt-runtime/tests/statement_parity.rs`
13. `keyed_fold_preview_matches_executed_statement_for_state_bearing_model` — the `smelt explain`
    preview for an `AVG` model carries the same state-column folds as the executed merge.

## Tasks

1. Spec + docs-site edits above.
2. `CrossPartitionCombiner::Recomputed` + `render` arm + doc comment (decision 1).
3. `classify_cumulative`'s `OtherAggregate` arm: when `combiner_for` returns `None`, attempt
   `decompose_to_state(sql_fn, distinct, &[arg_texts…], alias)` before refusing — admit on `Ok`
   with `Recomputed` + `state`, keep the existing `KeyedUnknownCombiner` on `Err`. Reuse
   `is_direct_function_call`; derive `distinct` from a leading `DISTINCT` in the argument text.
   Refuse the family under snapshot-reconcile via a new `snapshot_refusal_reason` arm
   (`decomposed fold`). The arm stays family-agnostic — any future encoded state shape widens
   with it.
4. `WindowedKeyedRule::refuse` and `ledger_grade` in `crates/smelt-runtime/src/cumulative.rs`:
   read `col.state` first (grade `Additive` iff any *state* column's combiner is `Sum`/`BitXor`;
   verify state columns' combiners instead of `combiner_for(per_partition_agg)`), falling back to
   the existing per-combiner logic for stateless columns.
5. `has_monoid_state_shape` in `analysis/decomposed_state.rs` + `faithful_fold` condition (2)
   widening (decision 2); `derive_fold_spec` mirrors the `ArgMax` precedent for the new family
   (arity 1, no `DISTINCT`, else refuse the whole derivation).
6. Move `expand_aggregator_column_folds` + `substitute_identifiers` from
   `smelt-runtime/src/cumulative.rs` into `smelt-logical`'s maintenance emit layer (single-owner
   statement rule) and have `smelt-runtime/src/diagnostics.rs`'s `Technique::KeyedFold` preview
   use it, augmenting its delta SQL with `state_augmented_projection` *before* compiling (the
   phase-5 ordering fix).
7. Update the phase-3/4 "always empty today / unreachable" comments in `execute.rs`,
   `compile.rs`, and the collision-detector call site that claim no family is state-bearing.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test keyed_families --test emit_statements --test walk_coverage`
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering --test keyed_reprocessed_window_refusal`
- `cargo test -p smelt-db --test maintenance_fold_spec_companion`
- `cargo test -p smelt-cli --test maintenance_conformance` — 47/47 must stay green and unchanged
  (no already-admitted spelling changes shape).

## Commit message

`feat(incremental): admit AVG/STDDEV/VAR keyed folds on hidden decomposed state`
