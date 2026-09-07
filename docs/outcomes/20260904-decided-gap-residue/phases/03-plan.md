# Phase 3 (reopened) — once-write route-2 `unique_key` skip + generative-pool witness

**Status:** planned · **Serves:** success criterion 3

## Objective

Implement the human decision (c) recorded in the outcome's Decision log: `classify_once_write`'s
route-2 candidate loop skips the declared-FD requirement when the candidate column is already a
member of the model's `unique_key`. That removes the validation wall that blocked the first
attempt at phase 3's generative-pool clause, so a `COALESCE(MAX(<key>), 0)` recipe becomes
declarable in the testkit and the once-write NULL schedule can cover the nullability route
end-to-end against real DuckDB.

## Spec delta (first — user-visible admission surface)

1. `docs/specs/incremental_shapes.md` §"The column-family catalogue", after the four
   once-write spellings: add one clause stating that in **any** reduction spelling a candidate
   column that is itself a member of the model's `unique_key` needs no declared functional
   dependency — key membership already establishes the per-key constancy the declaration would
   assert (the same argument the key-derived spelling makes). Amend the fallback-bearing
   spelling's trailing sentence "the functional dependency is still required" accordingly.
2. `docs/specs/incremental_shapes.md` §Known Divergences "The key grain": delete the
   multi-column-`unique_key` / no-generative-pool-witness clause (now closed). Keep the
   driving-clock-derived residual clause and the "bare reference, not an arbitrary key-derived
   expression" clause verbatim.
3. `docs-site/docs/reference/cumulative-aggregate.md` §"Once-write columns": in the
   multi-candidate paragraph ("Every candidate still needs its own declared functional
   dependency (or must itself be key-derived)"), name the case explicitly — a candidate that is
   one of the model's own `GROUP BY` key columns needs no declaration, `MAX`/`MIN` wrapper or
   not.

## Tests (red-green)

1. `smelt-logical` `rules::cumulative::tests::once_write_key_member_candidate_admits_without_a_declared_fd`
   — `COALESCE(MAX(id), 0)` with `unique_key = [id]` and **no** declared FD admits
   `Admitted { state: None }`. (This inverts and replaces
   `once_write_not_null_route_still_requires_the_functional_dependency`, which decision (c)
   supersedes — delete that test, do not weaken it in place.)
2. `smelt-logical` `rules::cumulative::tests::once_write_bare_key_reduction_admits_without_a_declared_fd`
   — `COALESCE(MAX(id))` (no fallback) with no FD also admits statelessly: the skip is in the
   candidate loop, so it covers every route-2 spelling, not just the fallback-bearing one.
3. `smelt-logical` `rules::cumulative::tests::once_write_non_key_candidate_still_requires_the_fd`
   — regression guard: `COALESCE(MAX(val), 0)` with no FD stays `Unproven`.
4. `smelt-db` `maintenance_fold_spec_companion::fold_spec_admits_the_key_member_candidate_without_a_declared_fd`
   — plan-layer/runtime admission parity for the FD-free route.
5. `smelt-cli` `maintenance_conformance::gate::once_write_key_fallback_pool_upholds_end_state_equivalence`
   — stages `KeyedCombiner::OnceWriteKeyFallback`, asserts the rendered model file contains no
   `functional_dependencies:` block, asserts a `Technique::KeyedFold` cell, and drives
   `once_write_constant_payload_schedule` through `drive_keyed_and_assert`.
6. `smelt-cli` `maintenance_conformance::gate::once_write_null_pool_upholds_end_state_equivalence`
   — extend the existing `combiners` array with `OnceWriteKeyFallback` so the generative NULL
   schedule covers the nullability route (criterion 3's own wording).

## Tasks

1. Make the spec + user-doc edits above (spec first).
2. `crates/smelt-logical/src/rules/cumulative.rs` — in `classify_once_write`'s
   `for (_, column) in &candidates` loop, `continue` when
   `analysis::not_null::column_provably_not_null(unique_key, column)`, before the `vector`
   resolution. Doc-comment it as the route-1 argument extended to a wrapped reference, and note
   it deliberately consults no `PropertyVector` (route 1 does not either).
3. Add tests 1–3; delete `once_write_not_null_route_still_requires_the_functional_dependency`.
4. Add test 4 to `crates/smelt-db/tests/maintenance_fold_spec_companion.rs`.
5. `crates/smelt-maintenance-testkit/src/recipe.rs` — add `KeyedCombiner::OnceWriteKeyFallback`
   (`kind_name` `"once_write_key_fallback"`, `agg_and_alias` `("COALESCE", "once_val")`,
   `ordering_alias` `None`); give `projection_sql` a `key` parameter and render
   `COALESCE(MAX({key}), 0) AS once_val` for the new arm. Keep it out of `arb_keyed_combiner`
   for the same world-fact reason as the other once-write variants.
6. `crates/smelt-maintenance-testkit/src/render.rs` — pass the key through both `projection_sql`
   call sites (`render_keyed_model_body`, the repair-recipe site ~L1262); leave the new variant
   **out** of `render_keyed_model_file`'s `fd_block` match — the absent FD block is the point.
7. Add tests 5–6 in `crates/smelt-cli/tests/maintenance_conformance/gate.rs`.
8. Rewrite `incremental_shapes.md`'s Known Divergences bullet per the spec delta once the
   witness is green (not before — the bullet stays honest until the test passes).

## Risks

- The recipe projects `id` twice (`SELECT id, COALESCE(MAX(id), 0) AS once_val … GROUP BY id`).
  The unit tests already use exactly this SQL shape, so the classifier accepts it; if an
  unrelated keyed-admission check refuses the double use at the plan layer, name the refusing
  check in the summary rather than working around it.

## Verification

- `cargo test -p smelt-logical --lib cumulative::`
- `cargo test -p smelt-db --test maintenance_fold_spec_companion`
- `cargo test -p smelt-cli --test maintenance_conformance once_write`
- `cargo test -p smelt-runtime --test statement_parity`
- `bash .claude/scripts/verify-phase.sh` (if the bundled script exceeds the foreground timeout,
  run its four legs separately — fmt-check, `clippy-gate.sh`, `cargo test`,
  `example_diagnostics` — as phase 6 did)

## Commit message

`feat(incremental): once-write route-2 skips the declared FD for a unique_key candidate`
