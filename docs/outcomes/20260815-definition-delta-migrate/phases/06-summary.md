# Phase 6 summary — close the atomicity divergence

**Shipped:**
- `schema_evolution::full_refresh_escape_requires_rebuild` (pure): a `schema_evolution: strategy:
  full_refresh` model whose deployed/inferred schemas diverge now forces a rebuild
  (`crates/smelt-runtime/src/schema_evolution.rs`); wired into `execute.rs`'s `use_alter == false`
  branch, replacing silent no-op.
- `schema_evolution::reconcile_add_columns` (pure): drops an `ADD COLUMN` statement whose column
  is already physically present, keeping its backfill `UPDATE`; wired into `check_and_migrate`
  just before executing the migration's `StatementGroup`, reading physical columns via
  `information_schema.columns` (portable across backends, unlike a zero-row `SELECT * LIMIT 0`
  projection — see Decisions).
- The standalone `execute_in_place_update` fallback dispatch in `execute.rs` is gone: a
  not-yet-folded backfill assignment now forces a full refresh (with `tracing::warn!`) when the
  table exists, and dispatches nothing when it doesn't. The now-dead
  `maintenance_driver::execute_in_place_update` executor was deleted (its only caller).
- `docs/specs/definition_deltas.md` §"The atomicity rule" is now unconditional; §"Boundary with
  schema_evolution.md" no longer describes a bypass; the "conditional in practice" Known
  Divergences bullet is removed.
- New tests: 2 unit tests for the pure fns, `partially_applied_migration_group_is_repaired_on_retry`
  (DuckDB, extends `schema_migration_backfill_atomicity.rs`), and a new e2e CLI test
  `crates/smelt-cli/tests/e2e/full_refresh_escape_rebuild.rs` covering the rebuild route
  end-to-end.

**Decisions:**
- Physical-column reconciliation reads `information_schema.columns`, not the plan's originally
  suggested `SELECT * FROM <table> LIMIT 0`: DuckDB's Arrow bridge returns **zero record batches**
  for a zero-row result set (confirmed by an instrumented run), so `LIMIT 0` cannot report a
  schema at all when the table is genuinely empty or after `LIMIT 0` short-circuits. This is
  recorded as an implementation choice, not a spec-level claim (the spec only promises the
  behavior, not the query shape).
- `full_refresh_escape_requires_rebuild` and the fallback's forced-refresh both route through the
  existing `force_full_refresh` flag / `plan.incremental.as_ref().filter(|_| !force_full_refresh)`
  gate already used by `should_force_full_refresh` — no new dispatch mechanism needed.

**For the next planner:**
- Phase 7 (diagnostic rename/split) and phase 8 (docs-site migration guide) are next per the
  table; neither depends on this phase's changes beyond the atomicity-rule spec text already
  being current.
- The new e2e test lives at `crates/smelt-cli/tests/e2e/full_refresh_escape_rebuild.rs` (registered
  in `tests/e2e/main.rs`) rather than extending `schema_migration_backfill_atomicity.rs` as the
  plan first suggested — that test needs the full `execute_project` pipeline (the full-refresh
  escape check lives in `execute.rs`, not `check_and_migrate`), which the CLI e2e harness already
  provides via `schema_evolution_incremental.rs`'s pattern.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-runtime --test schema_migration_backfill_atomicity` — 3 passed.
- `cargo test -p smelt-runtime --test statement_parity` — 23 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 74 passed.
- `cargo test -p smelt-cli --test migrate_apply` — 9 passed.
- `grep -n "atomicity rule is conditional" docs/specs/definition_deltas.md` — no match.
