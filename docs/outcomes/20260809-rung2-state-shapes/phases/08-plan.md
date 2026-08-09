# Phase 8 plan — conformance recipes for the decomposed-state families

## Objective

Give every family rows 5–7 newly admitted (`MAX_BY`/`MIN_BY`, once-write
fallback + multi-candidate, `AVG`/`STDDEV_*`) generative coverage in
`cargo test -p smelt-cli --test maintenance_conformance` against a real
DuckDB, and pair each state-bearing recipe with a downstream `SELECT *`
consumer model so state-column hiding is proven end-to-end rather than only
in unit tests. Advances success criteria 5 (recipes for every newly admitted
family) and 4 (state columns invisible downstream).

## Spec delta

None. This phase adds test coverage only; the user-visible semantics it
exercises were specified in rows 1–7 and no behaviour changes here. If an
implementation defect surfaces, fix the code — do not amend the spec without
saying so in the summary.

## Tests

Red-green, all in `crates/smelt-cli/tests/maintenance_conformance/gate.rs`
unless noted.

1. `decomposed_fold_pool_upholds_end_state_equivalence` — `AVG(val)` and
   `STDDEV_SAMP(val)` window-forward keyed recipes, driven through
   `drive_keyed_and_assert` over generated `arb_keyed_schedule` schedules,
   equal the `STracker` oracle after every window. Iterates the two
   combiners explicitly (not draw-dependent).
2. `once_write_fallback_pool_upholds_end_state_equivalence` — the
   `COALESCE(MAX(val), 0)` spelling over the once-write dedicated
   constant-payload schedule (mirrors
   `once_write_pool_upholds_end_state_equivalence`'s shape).
3. `once_write_multi_candidate_pool_upholds_end_state_equivalence` — the
   `COALESCE(MAX(val), MIN(val))` spelling, same schedule shape.
4. `state_bearing_recipes_physically_carry_state_columns` — for each new
   family plus `OrderMonotone`, `information_schema` on the maintained table
   shows at least one `__`-marked column. Guards the hiding assertions from
   being vacuous.
5. `downstream_select_star_consumer_sees_only_presented_columns` — for each
   state-bearing family, a staged downstream model `SELECT * FROM
   smelt.ref('<model>')` materialises with exactly the upstream's presented
   columns (no `__` names) and multiset-equals the upstream's presented
   contents after a real run.
6. `float_equivalence_comparison_tolerates_last_bit_only`
   (`.../harness_self_check.rs`) — the new float-aware comparison treats a
   1e-12 perturbation as equal and a 1e-3 perturbation as unequal, so the
   tolerance cannot silently swallow a real fold bug.

## Tasks

1. Add `KeyedCombiner` variants `DecomposedAvg` (`AVG(val) AS avg_val`),
   `DecomposedStddev` (`STDDEV_SAMP(val) AS stddev_val`),
   `OnceWriteFallback` (`COALESCE(MAX(val), 0) AS once_val`), and
   `OnceWriteMultiCandidate` (`COALESCE(MAX(val), MIN(val)) AS once_val`) in
   `crates/smelt-maintenance-testkit/src/recipe.rs`; fill `kind_name`,
   `agg_and_alias`, `ordering_alias`, `projection_sql`.
2. In `render.rs`, widen the once-write `fd_block` match from
   `KeyedCombiner::OnceWrite` to all three once-write variants; add
   `KeyedRecipe::new_window_forward_once_write_with(combiner)` (or widen the
   existing constructor to take the variant) so the dedicated
   constant-payload recipes reuse the same FD-backed staging.
3. Make the end-state comparison float-aware: derive the presented column
   list once from `information_schema` (name + data type) and build BOTH the
   maintained SQL and an oracle wrapper from it, wrapping `DOUBLE`/`FLOAT`/
   `REAL` columns in `ROUND(col, 6)`. Document why (DuckDB's `STDDEV_SAMP`
   uses a numerically stable pass; the decomposed `(n, Σx, Σx²)` recompute
   does not, so they agree only to ~1e-12).
4. Add tests 1–3; run them and fix any real classifier/emitter defect they
   expose (report, do not paper over).
5. Add downstream-consumer staging: an opt-in
   `stage_keyed_recipe_with_downstream` writing
   `models/<model>_downstream.sql` = `SELECT * FROM smelt.ref('<model>')`,
   plus an `assert_downstream_hides_state` helper. Add tests 4–5.
6. Add test 6 in `harness_self_check.rs`.
7. Extend `arb_keyed_combiner` with `DecomposedAvg` and `DecomposedStddev`
   so the main generative pool draws them too (the once-write variants stay
   excluded — their world-fact does not hold for that pool's data, same
   reason as `OnceWrite`). Re-run the full gate and record the new case
   count in the summary.
8. Refresh the now-stale `KeyedCombiner::OrderMonotone` doc comment (it
   still claims the ordering column must be projected as a companion `MAX`;
   row 5 deleted that obligation — the recipe keeps the projection as an
   ordinary output column).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test maintenance_conformance` — must stay green
  and now report more than 47 tests, with the new families named.
- `cargo test -p smelt-maintenance-testkit`

## Commit message

`test(incremental): conformance recipes + downstream SELECT * consumers for decomposed-state families`
