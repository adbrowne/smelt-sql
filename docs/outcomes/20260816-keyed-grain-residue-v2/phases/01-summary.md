# Phase 1 summary — Snapshot-reconcile anti-join delete leg

**Shipped:**
- `emit_snapshot_reconcile` (`crates/smelt-logical/src/maintenance/emit.rs`): pairs the
  existing keyed-fold `MERGE` (unconditional or suppressed, dispatched on `WriteSuppression`)
  with an anti-join `DELETE FROM <t> WHERE NOT EXISTS (SELECT 1 FROM (<scan>) AS s WHERE
  <key join>)`, returned as one transactional `StatementGroup`. No `slice` parameter — a
  snapshot-reconcile model reconciles the whole target by construction.
- `build_snapshot_reconcile_group` (`crates/smelt-runtime/src/cumulative.rs`): the
  combiner-rendering wrapper over the emitter, mirroring `build_cumulative_merge_sql`'s
  existing role but returning the full `StatementGroup` rather than a bare SQL string.
- `execute_snapshot_reconcile` now builds that group and executes it via
  `backend.execute_statement_group(&group)` — the same choke point `schema_evolution.rs` and
  `maintenance_driver.rs` already route every other maintenance statement group through.
  `build_cumulative_merge_sql` (used by the windowed-keyed executor) is untouched.
- `arb_snapshot_mutation_schedule` + `SnapshotMutation` (`smelt-maintenance-testkit/src/
  schedule_gen.rs`): deterministic-seeded insert/update/delete schedule generator over a small
  shared id pool; a `Delete`/`Update` drawn against a not-yet-live id repairs to an `Insert`
  rather than being rejected (mirrors `arb_keyed_schedule`'s "repair, don't reject" style).
- Tests: `emit_statements.rs` (3 new: group shape, composite-key conjunction, suppressed
  variant keeps the delete leg), `technique_lowering.rs` (`snapshot_reconcile_deletes_departed_key`
  replaces the old retention pin; `snapshot_reconcile_statements_come_from_the_emitter` is the
  statement-parity leg), `gate.rs` (`snapshot_reconcile_plain_overwrite_settles_after_key_departure`
  replaces the retained-departed variant; `snapshot_reconcile_pool_upholds_end_state_equivalence`
  is the new generative driver over the plain-overwrite/`ANY_VALUE` family — the only combiner the
  admission matrix admits under snapshot-reconcile).
- Doc fixes: `cumulative.rs`'s stale "carries no DELETE"/retained-unchanged doc comments now
  describe the anti-join delete; `oracle_modes.rs::keyed_end_state_with_retained_departed_keys`
  gets a doc line noting it is no longer the default-point comparator (kept as the future
  `retain_departed` quotient oracle's basis, phase 2).

**Decisions:**
- Reused `emit_keyed_fold`/`emit_keyed_fold_suppressed` internally inside `emit_snapshot_reconcile`
  rather than duplicating the `MERGE` shape, so the `MERGE` leg stays byte-identical by
  construction (asserted directly in the emit_statements test).
- The generative pool (test 7) drives only `KeyedCombiner::PlainOverwrite` — confirmed via
  `crates/smelt-logical/src/rules/cumulative.rs`'s `is_snapshot_reconcile` classification arm
  that every other combiner family (Additive, order-monotone, decomposed-fold) is refused
  fail-loud under snapshot-reconcile; there is no second admitted combiner to drive.

**For the next planner:**
- Phase 2 (`retain_departed` lattice point) can build directly on
  `oracle_modes::keyed_end_state_with_retained_departed_keys` as its quotient oracle transform —
  it is untouched pure data, just no longer wired to any default-point comparison.
- The generative pool test (`snapshot_reconcile_pool_upholds_end_state_equivalence`) runs
  `keyed_case_count()` (default 6) cases, each driving a full mutation schedule (up to 8 steps)
  through real `execute_project` runs — noticeably slower than the other keyed pool tests
  (~similar cost to `keyed_pool_upholds_end_state_equivalence`). If maintenance_conformance's
  wall-clock becomes a concern, this is the newest contributor.
- Observed one pre-existing, unrelated flake in `crates/smelt-runtime/tests/execute_parity.rs::
  test_cli_ui_manifest_parity_with_ephemerals` under full-workspace `cargo test` concurrency
  (passes reliably in isolation, both with and without this phase's changes) — not caused by
  this phase, not investigated further; worth a look if it recurs.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, workspace test, example_diagnostics)
- `cargo test -p smelt-logical --test emit_statements` — 48 passed
- `cargo test -p smelt-runtime --test technique_lowering --test statement_parity` — 23 + 31 passed
- `cargo test -p smelt-cli --test maintenance_conformance` — 84 passed
- `cargo test -p smelt-logical --test walk_coverage` — 4 passed
