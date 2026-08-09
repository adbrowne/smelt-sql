# Phase 3 plan — storage + emitters for decomposed state

## Objective

Make the state shapes phase 2 derives *physically real*: a state-bearing keyed column projects
its state columns into the stored table, and the keyed-fold `MERGE` combines each state column by
its own combiner and recomputes the presented column from the merged state. Wire
`KeyedStateColumnCollision`. This is the mechanism rows 5–6 flip admission onto; it advances
success criteria 1–3 (nothing admits without it) and 6 (gates stay green).

Admission is **not** widened here — no SQL that refuses today starts passing. Tests construct a
state-bearing `CumulativeClassification` directly.

## Spec delta

`docs/specs/incremental_models.md` §"Decomposed state (rung 2) in keyed models", the state-shape
catalogue table, once-write row, "Combiner over state" cell. It currently reads
"`written` is `OR`; `value` is the incumbent's unless the delta's `written` is true, in which
case the delta's" — last-write-wins, contradicting the family's first-write-wins semantics and
its own rung-1 `COALESCE(target, delta)` form (§"The column-family catalogue"). Replace with:
"`written` is `OR`; `value` is `COALESCE(target.value, delta.value)` — the incumbent's value
survives once written, the delta only ever fills a state row that was never written."

## Tests

`crates/smelt-logical/src/analysis/decomposed_state.rs` (unit):
- `avg_state_columns_carry_sum_combiners` — both `AVG` state columns fold additively.
- `arg_max_state_value_folds_on_ordering_column` — `v` folds order-monotone against the `o` state
  column; `o` folds `Max`.
- `arg_min_state_prefers_the_lesser_ordering` — `ARG_MIN`'s `v` fold wins on `delta.o < target.o`
  (red today: `OrderMonotone`'s render is unconditionally `>`), `o` folds `Min`.
- `once_write_state_value_keeps_the_incumbent` — `value` folds `OnceWrite`, `written` folds
  `BoolOr`.

`crates/smelt-logical/tests/emit_statements.rs`:
- `keyed_fold_over_state_projects_and_folds_state_columns` — a state-bearing fold set emits
  `SET <state cols> = <state combiners>, <presented col> = <π over the merged state exprs>`.
- `state_augmented_projection_appends_state_select_items` — the delta SELECT gains
  `, <per_partition_expr> AS <state col>` for each state column, key/GROUP BY untouched, the
  model's own presented select item unchanged.
- `state_augmented_projection_refuses_unparseable_sql` — refusal, not a mangled string.

`crates/smelt-logical/src/rules/cumulative.rs` (unit):
- `state_column_collision_is_diagnosed` — a projection aliased `spend__sum` alongside a
  state-bearing `spend` yields `KeyedStateColumnCollision` naming both names and the `__` suffix.
- `existing_keyed_classifications_carry_no_state` — every column family admitted today still
  classifies with `state: None` (the no-admission-widening guard).

`crates/smelt-runtime/src/cumulative.rs` (unit) / `tests/statement_parity.rs`:
- `build_cumulative_merge_sql_folds_state_columns` — a state-bearing classification produces a
  `MERGE` byte-identical to a direct `emit_keyed_fold` call over the state-expanded fold set.
- `stateless_merge_sql_is_unchanged` — no state ⇒ byte-identical to today's output.

## Tasks

1. Spec delta above (spec-first, one cell).
2. `CrossPartitionCombiner::OrderMonotone` gains a direction (`prefer_greater: bool`); `render`
   uses `>` / `<` accordingly; existing `MAX_BY` sites pass `true`, `MIN_BY` sites `false`.
3. `StateColumn` gains `combiner: CrossPartitionCombiner`; populate per family in
   `decomposed_state.rs` — `AVG`/variance components `Sum`; `ArgMax`/`ArgMin` `v` →
   `OrderMonotone { ordering_column: <o col>, prefer_greater }`, `o` → `Max`/`Min`; once-write
   `value` → `OnceWrite`, `written` → `BoolOr`. Reuse the existing enum rather than introducing a
   second combiner vocabulary.
4. `AggregatorColumn` gains `state: Option<DecomposedState>`, defaulting `None` at every current
   construction site.
5. New pure emitter in `smelt-logical`'s maintenance layer (statement single-owner rule):
   `state_augmented_projection(sql, &[StateColumn]) -> Result<String, _>` — parse the SELECT and
   append `<per_partition_expr> AS <name>` select items; refuse (never text-splice blind) when
   the select list cannot be located. No whole-text scan: locate the select list via the CST.
6. In `build_cumulative_merge_sql`, expand each state-bearing `AggregatorColumn` into: one fold
   pair per state column (its own combiner rendered over `target.<c>`/`delta.<c>`), plus the
   presented column set to the presentation expression with each state-column reference
   substituted by that column's *merged* expression. Stateless columns keep today's single pair.
7. Call `state_augmented_projection` on the compiled delta SQL in `execute_cumulative_aggregate`
   and `execute_snapshot_reconcile` before it reaches create-table-as / the `MERGE`, so the stored
   table's columns and the delta's columns agree (`WHEN NOT MATCHED THEN INSERT *` depends on it).
8. Add `KeyedDiagnostic::KeyedStateColumnCollision { state_column, user_column }`; derive it in
   `classify_cumulative` from `state_column_collisions` over all derived state columns × all
   projection aliases; render it in `format_classifier_error` (runtime) and the `smelt-db`
   maintenance diagnostic mapping, message naming both columns and the reserved `__` suffix.
9. Update the `DecomposedState`/`StateColumn` doc comments to cite the corrected spec cell.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test emit_statements --test walk_coverage`
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity`
- `cargo test -p smelt-cli --test maintenance_conformance` (must stay green — phase 3 widens no
  admission, so no recipe's verdict may change)

## Commit message

`feat(incremental): materialise decomposed state columns and fold them in the keyed merge`
