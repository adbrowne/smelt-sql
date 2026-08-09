# Phase 3 summary — storage + emitters for decomposed state

**Shipped:**
- `StateColumn` (`crates/smelt-logical/src/analysis/decomposed_state.rs`) carries its own
  `combiner: CrossPartitionCombiner`, populated per family: `AVG`/variance components `Sum`;
  `ArgMax`/`ArgMin`'s `v` → `OrderMonotone { ordering_column, prefer_greater }`, `o` → `Max`/`Min`;
  once-write `value` → `OnceWrite`, `written` → `BoolOr`.
- `CrossPartitionCombiner::OrderMonotone` gained `prefer_greater: bool`; `render` uses `>`/`<`
  accordingly. All existing `MAX_BY` call sites pass `true`; the new `MIN_BY`-shaped call site
  passes `false` (previously unconditionally `>`, silently wrong for `MIN_BY`).
- `AggregatorColumn` gained `state: Option<DecomposedState>`, `None` at every current
  construction site (36 sites across the workspace) — admission stays exactly where it was.
- `KeyedDiagnostic::KeyedStateColumnCollision` + `diagnose_state_column_collisions` (pure
  detector, `crates/smelt-logical/src/rules/cumulative.rs`), wired into `classify_cumulative` and
  through `RuleDiagnosticCode`/`DiagnosticCode`/LSP code-string mapping. Unreachable today (no
  column classifies with `state: Some`) — inherited for free once rows 5-6 widen admission.
- New pure emitter `state_augmented_projection` (`smelt-logical`'s maintenance layer,
  `maintenance/emit.rs`): appends `, <per_partition_expr> AS <state col>` select items via CST
  location of the last select item's `text_range` (never a whole-text scan), refusing
  (`StateAugmentRefusal::Unparseable`) rather than mangling unparseable SQL.
- `smelt-runtime`'s `build_cumulative_merge_sql` expands a state-bearing `AggregatorColumn` into
  one fold pair per state column plus the presented column re-derived from the *merged* state
  exprs (`expand_aggregator_column_folds` + `substitute_identifier`, a word-boundary-safe
  identifier substitution). `execute_cumulative_aggregate`/`execute_snapshot_reconcile` call
  `state_augmented_projection` on the compiled delta SQL before `CREATE TABLE AS`/`MERGE`.
- Spec: `docs/specs/incremental_models.md`'s once-write "Combiner over state" cell corrected from
  last-write-wins to `COALESCE(target.value, delta.value)` (first-write-wins, matching the
  family's rung-1 form).

**Decisions:**
- 2026-08-09: `diagnose_state_column_collisions` is wired into `classify_cumulative` now, even
  though it is unreachable (every family still classifies `state: None`) — avoids a second
  wiring pass in rows 5-6 and keeps the collision check adjacent to the classifier that will
  eventually populate `state`.
- 2026-08-09: `expand_aggregator_column_folds`/`substitute_identifier` live in `smelt-runtime`
  (not `smelt-logical`) — they render `CrossPartitionCombiner` to SQL text, the same
  layering `build_cumulative_merge_sql` already followed pre-phase (`smelt-logical` never
  depends on `smelt-planner`/combiner-rendering logic).

**For the next planner:**
- Row 4 (presentation projection) is the next phase per the outcome's existing reshape — hiding
  state columns from `ref()`/`SELECT *`/declared-schema checks/downstream type inference lives in
  `smelt-db`/`smelt-runtime` schema resolution, untouched by this phase.
- Rows 5-6 (admission) can now populate `AggregatorColumn.state` from `decompose_to_state`/
  `decompose_once_write` and the `MERGE`/storage machinery will fold it correctly — verified here
  only via directly-constructed classifications (`build_cumulative_merge_sql_folds_state_columns`,
  `state_column_collision_is_diagnosed`), since admission is not yet widened.
- No follow-up work was found out of scope for this phase; the `MIN_BY` render bug
  (unconditional `>`) was fixed as part of task 2, not deferred.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings, full `cargo test`
  workspace, example_diagnostics).
- `cargo test -p smelt-logical --test emit_statements --test walk_coverage` — pass (23 + 4 tests).
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity` — pass (18 + 4).
- `cargo test -p smelt-cli --test maintenance_conformance` — pass (47 tests, unchanged verdicts —
  no admission widened).
