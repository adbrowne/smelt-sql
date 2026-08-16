# Phase 7 summary — close the atomicity divergence

## Shipped

- `DefinitionChangeRoute` (`AtomicGroup` / `FullRebuild` / `Refuse { message }`) and the pure
  `resolve_definition_change_route(strategy, supports_transactional_ddl, has_pending_column_add,
  allow_full_refresh)` in `crates/smelt-runtime/src/schema_evolution.rs` — the single routing
  decision `docs/specs/definition_deltas.md` §"The atomicity rule" now names.
- `execute.rs`'s schema-evolution gate calls the route once per model, replacing the old
  `use_alter` boolean: `AtomicGroup` runs today's `check_and_migrate` path unchanged;
  `FullRebuild` sets `force_full_refresh = true` (no ALTER, no standalone backfill — the model's
  ordinary full-refresh path recomputes every row from the new definition); `Refuse` returns
  `Err` naming `--allow-full-refresh` before anything is written.
- Deleted the standalone `execute_in_place_update` **fallback call site** in `execute.rs` (the
  emitter itself is untouched — `smelt migrate --apply` and the maintenance driver still own it).
  `used_in_place_update` (a strategy-label input) is now set directly from whether the migration
  group folded the backfill, not from a separate dispatch.
- `has_pending_column_add` is computed from an actual schema diff (`diff_schemas` against the
  loaded deployed snapshot), not just presence of the `InPlaceUpdate` cell — so a
  `full_refresh`-strategy model's previously-silent bug (declared "always rebuild" not honoured
  at all) is fixed for every kind of schema change, not only backfill-needing column adds.
- Spec: `docs/specs/definition_deltas.md` §"The atomicity rule" states the rule unconditionally
  and names all three routes; §"Boundary with `schema_evolution.md`" no longer calls the
  `full_refresh` escape a bypass; the "atomicity rule is conditional" Known Divergences bullet is
  deleted. `docs/specs/schema_evolution.md` gained §"Routing on a maintained model".
  `docs-site/docs/guide/schema-evolution.md` updated (strategy table row + a note).
- Tests: 6 new unit tests (`resolve_definition_change_route`, pure) + 2 new integration tests in
  `crates/smelt-runtime/tests/schema_migration_backfill_atomicity.rs` driving the real
  `execute_project` entry point (not `check_and_migrate` directly) — one exercising the
  `full_refresh`-strategy rebuild route, one pinning the default atomic-group route unchanged.

## Decisions

- `has_pending_column_add`'s caller-side meaning is "there is a schema diff for this model this
  run" (loaded deployed schema vs freshly inferred columns), computed once before routing — not
  narrowly "the `InPlaceUpdate` cell resolved". This makes `route_is_atomic_group_when_there_is_no_
  pending_column_add`'s "ordinary run" framing literally true, and fixes the `full_refresh`
  strategy's declared-but-unhonoured rebuild intent for non-backfill changes too (in scope per the
  outcome's own decision-log framing of the bug).
- The diff is computed twice (once for routing, once inside `check_and_migrate` on the
  `AtomicGroup` path) rather than threading it through — `diff_schemas` is a cheap in-memory
  column comparison; avoiding the double call would have required restructuring
  `check_and_migrate`'s signature for no real cost saving.

## For the next planner

- Discovered while writing the integration tests (not a phase-7 regression): the derived
  `InPlaceUpdate` backfill expression carries the model SQL's own FROM-alias verbatim (e.g.
  `e.val * 2`), which is invalid inside the folded `UPDATE ... SET` (no `FROM e` there) —
  `resolve_live_in_place_update_cell` needs to strip/rebind the alias, or the caller needs to
  normalize it, for any aliased single-table FROM clause. Every existing fixture in the repo
  (including this phase's own) avoids the alias to route around it; a real user model that
  aliases its FROM table would hit this. Not touched here — pre-existing, orthogonal to the
  atomicity routing this phase closes. Worth a follow-up phase or its own bug fix.
- Phase 8 (diagnostic rename) and phase 9 (docs-site migration guide) are next in the table;
  neither is blocked by this phase's changes.

## Gates

- `bash .claude/scripts/verify-phase.sh` — PASS (fmt, clippy zero-warnings, full `cargo test`,
  `example_diagnostics`)
- `cargo test -p smelt-runtime --test schema_migration_backfill_atomicity --quiet` — 4 passed
- `cargo test -p smelt-runtime --test statement_parity --quiet` — 23 passed
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb --quiet` — 83 passed
- `cargo test -p smelt-cli --test migrate --features duckdb --quiet` — 14 passed
- `cargo check -p smelt-cli --tests --features spark` — clean
