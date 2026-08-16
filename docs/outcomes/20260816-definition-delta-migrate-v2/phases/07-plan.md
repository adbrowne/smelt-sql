# Phase 7 plan — close the atomicity divergence

## Objective

Make `definition_deltas.md` §"The atomicity rule" hold unconditionally: no run may ever apply a
definition change as a separately-dispatched `ADD COLUMN` + standalone `UPDATE`. The two escapes
the divergence bullet names — a model declaring `schema_evolution: strategy: full_refresh`, and a
backend without transactional DDL — are unified into one routing decision with a real repair path,
and the divergence bullet is removed. Advances success criterion 5 (and criterion 8's
"bullets removed, not just addressed in code").

## The decision this phase implements

One pure route decision replaces the current implicit fall-through:

| model strategy | backend transactional DDL | route |
|---|---|---|
| `alter_and_backfill` (default) | yes | `AtomicGroup` — today's `check_and_migrate` `StatementGroup` (unchanged) |
| `full_refresh` | either | `FullRebuild` — the declaration *is* the consent; the table is rebuilt from the new definition |
| `alter_and_backfill` | no | `FullRebuild` if `--allow-full-refresh`, else `Refuse` — error naming `smelt run --full-refresh <model>` / `--allow-full-refresh` as the recovery |

`Refuse` is the repair path for the non-transactional case: nothing is applied, the recorded
definition is untouched, so the next invocation re-derives the identical change. The standalone
`execute_in_place_update` **fallback call site** in `execute.rs` is deleted; the emitter itself
stays (`smelt migrate --apply` and the maintenance driver still own it).

## Spec delta (implement step makes these edits first)

- `docs/specs/definition_deltas.md` §"The atomicity rule" — state the rule unconditionally and name
  the two non-`AtomicGroup` routes (full rebuild / refusal with recovery), explicitly forbidding a
  separately-dispatched backfill on any backend.
- `docs/specs/definition_deltas.md` §"Boundary with `schema_evolution.md`" — replace the
  "currently bypasses that gate — a recorded divergence" parenthetical with the unified rule:
  `strategy: full_refresh` does not escape the atomicity rule, it selects the rebuild route.
- `docs/specs/definition_deltas.md` §Known Divergences — delete the
  "**The atomicity rule is conditional in practice.**" bullet entirely.
- `docs/specs/schema_evolution.md` — record that on a maintained (incremental) model
  `strategy: full_refresh` rebuilds on a detected change rather than skipping migration, and that a
  backend without transactional DDL never takes the `ALTER` + `UPDATE` two-step.
- `docs-site/docs/guide/schema-evolution.md` — the strategy table row (line ~43) and one prose
  sentence, matching the spec wording.

## Tests (red-green, in this order)

Unit — new `route` module in `crates/smelt-runtime/src/schema_evolution.rs` (pure fn, no backend):

1. `route_default_strategy_transactional_backend_is_atomic_group` — the unchanged happy path.
2. `route_full_refresh_strategy_is_full_rebuild` — declared strategy selects the rebuild route
   regardless of backend capability or `--allow-full-refresh`.
3. `route_non_transactional_backend_refuses_without_opt_in` — returns `Refuse`.
4. `route_non_transactional_backend_with_allow_full_refresh_is_full_rebuild`.
5. `refusal_message_names_the_recovery_flag` — the `Refuse` message contains
   `--allow-full-refresh` (fail-loud: the user is told what to run).
6. `route_is_atomic_group_when_there_is_no_pending_column_add` — a run with no definition change
   never routes to a rebuild (regression guard: this must not fire on ordinary runs).

Integration — `crates/smelt-runtime/tests/schema_migration_backfill_atomicity.rs` (DuckDB):

7. `full_refresh_strategy_rebuilds_and_never_dispatches_a_standalone_backfill` — an incremental
   model with `schema_evolution: strategy: full_refresh` plus an added column: after the run every
   row carries the new column's value, the row count is unchanged (no duplication), and the stored
   schema advanced — driven through `execute_project`, not `check_and_migrate` directly.
8. `default_strategy_still_folds_the_backfill_into_the_migration_group` — the existing
   `derived_backfill_folds_into_migration_group_and_backfills_every_row` must still pass unchanged
   (assert, don't rewrite): deleting the fallback call site changes nothing on the atomic path.

## Tasks

1. Apply the spec + docs-site edits above (spec-first).
2. Add `DefinitionChangeRoute { AtomicGroup, FullRebuild, Refuse { message } }` and a pure
   `resolve_definition_change_route(strategy, supports_transactional_ddl, has_pending_column_add, allow_full_refresh)`
   to `crates/smelt-runtime/src/schema_evolution.rs`; write tests 1–6 red first.
3. Call it once in `execute.rs` at the schema-evolution gate: `AtomicGroup` keeps today's
   `check_and_migrate` path; `FullRebuild` sets `force_full_refresh = true` and skips the gate;
   `Refuse` returns `Err` with the message (no writes).
4. Delete the standalone `execute_in_place_update` fallback dispatch block in `execute.rs`
   (and the now-dead `migration_backfilled_columns` skip bookkeeping if it becomes unused); keep
   `used_in_place_update` correct for the atomic path.
5. Fix the stale comment pointer at the deleted block's site — it cited
   `incremental_models.md` §Known Divergences, which carries no such bullet.
6. Add integration tests 7–8; confirm no other call site of the deleted block regressed
   (`rg execute_in_place_update`).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-runtime --test schema_migration_backfill_atomicity --quiet`
- `cargo test -p smelt-runtime --test statement_parity --quiet`
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb --quiet 2>&1 | tail -20`
- `cargo test -p smelt-cli --test migrate --features duckdb --quiet`
- `cargo check -p smelt-cli --tests --features spark` (Spark is the non-transactional backend this
  phase reroutes; the gated suite must still compile)

## Commit message

`fix(runtime): route definition changes that cannot be applied atomically to a rebuild or refusal`
