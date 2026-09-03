# Phase 6 plan — close the atomicity divergence

## Objective

Make `definition_deltas.md` §"The atomicity rule" unconditional, advancing success criterion 5.
Two escapes exist today: a model declaring `schema_evolution: strategy: full_refresh` skips the
migration gate entirely and falls through to the standalone (non-atomic, and on an unmigrated
table outright broken) `execute_in_place_update` dispatch; and on a backend without transactional
DDL a partially-applied `ADD COLUMN` + backfill group has no retry route, because the retry
re-emits an `ADD COLUMN` for a column that is now physically present. Both are closed here — the
first by unifying the escape with the gate (a definition change on a `full_refresh`-strategy model
rebuilds the table, atomic by construction), the second by a real repair path (reconcile the
migration group against the target's physical columns before executing it).

## Design decision (record in the spec, not only the code)

- **The `full_refresh` escape is unified, not given a parallel repair path.** A model whose
  `schema_evolution: strategy: full_refresh` opts out of `ALTER`-based evolution now *actually
  full-refreshes* when its schema changed, instead of silently taking neither route. This is the
  option success criterion 5 offers first, and it needs no new machinery.
- **The derived in-place backfill is only ever dispatched inside the migration statement group.**
  The standalone dispatch is removed as a *fallback*: with the table absent there are no rows to
  backfill; with the gate skipped or unable to fold the column, the model force-full-refreshes
  (always correct, always atomic) and says so via `tracing::warn!` + the reporter, rather than
  issuing an `UPDATE` against a schema that may not carry the column.
- **Repair path for non-transactional DDL:** before executing a migration group, the derived
  `ADD COLUMN` statements are reconciled against the target's *physical* columns (read portably
  via `SELECT * FROM <table> LIMIT 0` through `Backend::execute_sql`, whose returned batch schema
  names them). A column already physically present has its `ADD COLUMN` dropped and its backfill
  `UPDATE` kept — so a group that partially applied and then failed completes on the next run.
  Reconciliation runs on every backend (not gated on `supports_transactional_ddl`), because it is
  one cheap query per *actual* migration and it also repairs snapshot/physical drift generally.

## Spec delta (implement first)

`docs/specs/definition_deltas.md`:
1. §"The atomicity rule" — state the two rules above: the derived backfill is emitted only inside
   the migration's own statement group, never as a separately-dispatched write; a model opting out
   via `schema_evolution: strategy: full_refresh` rebuilds under the new definition rather than
   taking a two-step; and on a backend without transactional DDL the group is made rerun-safe by
   physical-column reconciliation, so a partial application is repaired by the next run rather
   than wedging. Keep it timeless (no phase/plan vocabulary).
2. §"Boundary with `schema_evolution.md`" — delete the trailing parenthetical claiming the
   `strategy: full_refresh` escape bypasses the gate; replace with one sentence saying the escape
   routes to a rebuild under the definition-delta path's own admission.
3. §Known Divergences — remove the whole "**The atomicity rule is conditional in practice**"
   bullet.
4. If `docs/specs/schema_evolution.md` describes the `strategy: full_refresh` knob anywhere, add
   one sentence cross-referencing the rebuild behaviour on an incremental model; if it does not,
   change nothing there.

## Tests (red-green, in this order)

- `smelt-runtime` unit, `schema_evolution.rs`: `reconcile_add_columns_skips_physically_present_column`
  — the new pure reconciliation fn drops an `ADD COLUMN` whose column is already present and keeps
  the matching backfill `UPDATE`.
- `smelt-runtime` unit: `reconcile_add_columns_is_identity_when_nothing_present` — no behaviour
  change on the ordinary (nothing partially applied) path.
- `crates/smelt-runtime/tests/schema_migration_backfill_atomicity.rs` (extend):
  `partially_applied_migration_group_is_repaired_on_retry` — DuckDB: add the column by hand behind
  a stale saved snapshot, then `check_and_migrate` succeeds and the column is backfilled non-NULL
  (today it fails with "column already exists").
- Same file: `full_refresh_strategy_never_dispatches_standalone_backfill` — a `full_refresh`
  -strategy incremental model with an added column takes the rebuild route; assert the standalone
  in-place `UPDATE` is not issued and the rebuilt table carries the column populated.
- `smelt-runtime` unit over the new admission helper:
  `full_refresh_escape_forces_rebuild_only_when_schema_changed` — an unchanged `full_refresh`
  -strategy model still runs incrementally (no gratuitous rebuild).

## Tasks

1. Land the spec delta above (spec-first).
2. Add `schema_evolution::reconcile_add_columns(statements, physical_columns) -> Vec<String>` as a
   pure fn with the two unit tests; call it inside `check_and_migrate` just before building the
   `StatementGroup`, reading physical columns via `execute_sql("SELECT * FROM … LIMIT 0")`
   (skip the read on `dry_run`).
3. Add `schema_evolution::full_refresh_escape_requires_rebuild(strategy, deployed, inferred) -> bool`
   (pure) and call it in `execute.rs` where `use_alter == false` today, setting `force_full_refresh`.
4. In `execute.rs`, remove the standalone `execute_in_place_update` fallback for the residual
   cases: when assignments remain after the migration gate and the table exists, force a full
   refresh with a `tracing::warn!` naming the reason instead of issuing the `UPDATE`; when the
   table does not exist, dispatch nothing. Update the surrounding comment block, which currently
   points at the divergence being removed.
5. Check whether `maintenance_driver::execute_in_place_update` still has a live caller; if not,
   delete it (and its tests) rather than leaving a dead non-atomic entry point — the
   statement-emission single-owner rule makes a dead executor a trap.
6. Re-run the standing gates and fix fallout (`statement_parity` and `maintenance_conformance` both
   exercise this path).

## Verification

- `bash .claude/scripts/verify-phase.sh` (full)
- `cargo test -p smelt-runtime --test schema_migration_backfill_atomicity`
- `cargo test -p smelt-runtime --test statement_parity`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-cli --test migrate_apply`
- `grep -n "atomicity rule is conditional" docs/specs/definition_deltas.md` returns nothing.

## Commit message

`fix(runtime): make the definition-delta atomicity rule unconditional`
