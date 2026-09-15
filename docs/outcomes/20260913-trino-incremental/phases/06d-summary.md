# Phase 6d summary — the live-Trino conditional-write parity failure

## Shipped

- **Fail-loud `table_exists` checks.** All 9 `table_exists(...).unwrap_or(false)`
  sites in `smelt-runtime` (`maintenance_driver/driver.rs`,
  `maintenance_driver/succession/execute.rs` x2, `cumulative.rs`,
  `execute/project/mod.rs` x6) now propagate the backend's own error via
  `.with_context(...)?` instead of masking it to "table does not exist".
  New tests: `maintenance_driver::tests::first_run_check_propagates_a_backend_error`
  (unit, `RecordingBackend` gains a `table_exists_error` fault-injection field)
  and `crates/smelt-runtime/tests/table_exists_error_census.rs` (source-scan
  census — no site can regress silently).
- **Fixed a genuine double-dispatch defect** in `execute/project/mod.rs`: the
  membership-recompute dispatch (`used_membership_recompute` arm, ~line 2417)
  now also requires `whole_target_rebuild_downgrade.is_none()`. When a model's
  `NewData`-trigger cell has been downgraded to an unconditional whole-target
  rebuild (`keyed_fold_state_downgrade`/`repair_state_downgrade`, e.g. because
  Trino has no reconciliation ledger), that rebuild already recomputes the
  model's entire current admitted state — running the narrower
  membership-recompute cell alongside it produced two executed statement
  groups for one logical write.

## Decisions

- The plan's leading hypothesis (a masked `BackendError` from a fresh
  `TrinoBackend`'s `table_exists` query, collapsed to `false`) was **measured
  and falsified**: a tight in-process probe (fresh `TrinoBackend`, 0ms–1000ms
  delays) showed `information_schema.tables` visibility is immediate and
  consistent on this tier every time — no REST-catalog visibility window
  exists here. The masking was still worth removing (fail-loud discipline,
  §CLAUDE.md), and its unit/census tests land, but it was not the cause of
  the live failure.
- Instrumented the real dispatch (temporary `eprintln!`s, removed before
  commit) and found the actual cause: `whole_target_rebuild_downgrade`
  (`keyed_fold_state_downgrade.or(repair_state_downgrade)`) is `Some` on
  **every** run of this fixture's model on Trino (its `NewData`-trigger cell
  is `Technique::KeyedFold`, permanently downgraded because Trino has no
  reconciliation ledger), and that downgrade's drop+create fires
  unconditionally — regardless of `table_exists_before_run` — alongside the
  membership-recompute dispatch, which fires whenever
  `table_exists_before_run` is true. Run 1 (`table_exists_before_run == false`)
  only hits the downgrade path; run 2 hits both. Fixed by suppressing
  membership recompute when a whole-target rebuild already covers the run.
- Appended to outcome.md's Decision log (dated).

## For the next planner

- **The live test `staged_candidate_conditional_parity_on_trino` still fails**,
  now for a different, well-understood reason: with double-dispatch fixed,
  run 2 executes exactly one group — but it's the whole-target rebuild
  (`CREATE TABLE ... AS ...`), not the staged-candidate conditional recompute
  the test asserts against. This fixture's aggregate (`COUNT` over a fact
  joined to a mutable dimension for row admission) is, by the classifier,
  necessarily *also* an additive-`KeyedFold`-eligible cell for its `NewData`
  trigger — and on Trino that cell is permanently downgraded to a whole-target
  rebuild (already correctly proven by the passing
  `additive_keyed_fold_downgrade_parity_on_trino`), which subsumes anything
  membership recompute would do. So **this exact fixture can never exercise
  the staged-candidate write live on Trino** — not a masked bug, a structural
  consequence of Trino's missing reconciliation ledger.
  - Also worth noting: `resolve_keyed_fold_state_downgrade` and
    `resolve_repair_state_downgrade` (`maintenance_driver/resolve/live_cells.rs`)
    do not accept `technique_overrides` at all (unlike
    `resolve_live_membership_recompute_cell`, which does but only as a
    post-filter over an already-derived plan). A `technique_overrides` pin
    forcing this source's technique away from `KeyedFold` would not currently
    stop the downgrade resolvers from independently deriving and downgrading
    a competing `KeyedFold` cell for the same source.
  - Options for whoever re-plans this: (a) thread `technique_overrides`
    through the two downgrade resolvers so a pin can suppress the competing
    cell, then give this fixture such a pin; (b) reshape the fixture to a
    model whose `NewData` trigger is not `KeyedFold`-eligible at all (may not
    be possible under today's classifier per the doc comment in
    `keyed_membership_recompute_e2e.rs`); (c) accept the whole-target-rebuild
    outcome as correct for this fixture on Trino and rewrite the test's
    assertions to check *that* route's parity, moving the staged-candidate
    emitter's Trino byte-identity proof to a differently-shaped model or to a
    non-`execute_project` unit-level harness.
  - Whichever option is chosen, re-run 6d's Verification block afterward.
- The `whole_target_rebuild_downgrade` vs. `column_scoped_cell` dispatch
  (line ~2222, gated only on `table_exists_before_run`) has the exact same
  shape as the bug just fixed for membership recompute. No test currently
  proves column-scoped merge is dispatched alongside a whole-target-rebuild
  downgrade, so it is unverified either way — worth a targeted check before
  it surfaces as a second live-Trino surprise.

## Gates

- `cargo fmt --all -- --check` — clean.
- `cargo clippy -p smelt-runtime --all-targets -- -D warnings` — clean.
- `cargo test -p smelt-runtime --test statement_parity --test execute_parity --test dry_run_statements --lib --quiet` — all green (no `SMELT_TRINO_URL`, so the live Trino legs are skipped in this run).
- `cargo test -p smelt-cli --test trino_ci_wiring --quiet` — green.
- `cargo test -p smelt-runtime --test table_exists_error_census` — green (new).
- `cargo test -p smelt-runtime --lib maintenance_driver::tests` — green, including the new `first_run_check_propagates_a_backend_error`.
- `cargo test -p smelt-runtime --test technique_lowering --lib` (DuckDB e2e membership/keyed-fold suite) — green, confirming no regression to the non-Trino dispatch path.
- **Live tier** (`scripts/trino-up.sh` / `scripts/trino-env.sh`):
  `cargo test -p smelt-runtime --test statement_parity -- --test-threads=1` — 45/46 Trino legs green; `trino::staged_candidate_conditional_parity_on_trino` still **fails** for the structural reason above. Torn down with `scripts/trino-down.sh`.
