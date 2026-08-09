# Phase 8 summary — conformance recipes for the decomposed-state families

## Shipped

- `KeyedCombiner` gained `DecomposedAvg`/`DecomposedStddev`/`OnceWriteFallback`/
  `OnceWriteMultiCandidate` (`crates/smelt-maintenance-testkit/src/recipe.rs`);
  `arb_keyed_combiner()` now draws the decomposed-fold pair too. `render.rs`'s
  `fd_block` match widened to all three once-write variants;
  `KeyedRecipe::new_window_forward_once_write_with(combiner)` generalises the
  existing once-write constructor.
- `assert_keyed_equivalence`/`presented_columns_with_types`/`rounded_select_list`
  (`crates/smelt-cli/tests/maintenance_conformance/gate.rs`) make the keyed
  end-state comparison float-aware: one `information_schema`-derived
  `(name, data_type)` list builds BOTH the maintained and oracle selects,
  `DOUBLE`/`FLOAT`/`REAL` columns wrapped `ROUND(col, 6)`.
- Tests 1-3: `decomposed_fold_pool_upholds_end_state_equivalence` (AVG/STDDEV
  over generated `arb_keyed_schedule`), `once_write_fallback_pool_...`,
  `once_write_multi_candidate_pool_...` (both over a shared
  `once_write_constant_payload_schedule()`). All passed on first run — no
  classifier/emitter defect surfaced.
- `render::stage_keyed_with_downstream` + `gate.rs`'s
  `stage_keyed_recipe_with_downstream`/`assert_downstream_hides_state`/
  `all_physical_column_names`: opt-in downstream `SELECT * FROM
  smelt.<model>` consumer staging. Tests 4-5:
  `state_bearing_recipes_physically_carry_state_columns` (vacuity guard),
  `downstream_select_star_consumer_sees_only_presented_columns`.
- Test 6: `float_equivalence_comparison_tolerates_last_bit_only`
  (`harness_self_check.rs`) pins the ROUND(6) tolerance (1e-12 passes,
  1e-3 fails).
- `KeyedCombiner::OrderMonotone`'s doc comment refreshed (row 5 already
  deleted the companion-`MAX`-projection obligation; comment now says
  hidden `(v, o)` state).

## Decisions

- Ref syntax bug found and fixed: the plan's own text (`SELECT * FROM
  smelt.ref('<model>')`) uses a removed syntax
  (`smelt.ref()`/`smelt.source()` were deleted in favour of
  `smelt.models.<name>`/`smelt.sources.<schema>.<table>`). Neither of those
  worked for a same-project model ref either — the parser's own suggestion
  (`did you mean 'smelt.<name>'`) pointed at the shorthand `dag.rs` already
  uses; `stage_keyed_with_downstream` renders `SELECT * FROM smelt.<model>`.
- `once_write_constant_payload_schedule()` factored out of the pre-existing
  `once_write_pool_upholds_end_state_equivalence` so all three once-write
  tests share one literal schedule, per the plan's "mirrors ... shape"
  instruction.

## For the next planner

- No classifier/emitter defects surfaced by the new recipes — `MAX_BY`/
  once-write/AVG/STDDEV all folded correctly against the oracle on the
  first run. Row 9 (surface cleanup, `smelt explain` state rendering) is
  the only work left on the outcome.
- New case count: `cargo test -p smelt-cli --test maintenance_conformance`
  now reports 53 tests (was 47); `keyed_case_count()`'s env-tunable
  `SMELT_CONFORMANCE_KEYED_CASES` (default 6) also governs the new
  `decomposed_fold_pool_upholds_end_state_equivalence` loop.

## Gates

- `bash .claude/scripts/verify-phase.sh` — green (fmt, clippy, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-cli --test maintenance_conformance` — 53/53 green.
- `cargo test -p smelt-maintenance-testkit` — 29/29 green.
