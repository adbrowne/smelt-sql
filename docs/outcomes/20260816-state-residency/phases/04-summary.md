# Phase 4 summary — the reconciliation ledger goes engine-resident

## Shipped

- `_smelt_frontier` table (`crates/smelt-state/src/ddl_duckdb.rs`): DDL, region-intersecting
  reset `DELETE`, and insert, keyed `(model_name, grp, region_start)` — mirrors `_smelt_ledger`'s
  shape, one row per region-recompute (matches the only shape any caller writes: a single
  `"self"` input per region).
- `Backend::execute_write_and_reset_frontier` (`crates/smelt-backend/src/lib.rs`) with a
  non-atomic default; `smelt-backend-duckdb` overrides it with a real single-transaction
  implementation (`crates/smelt-backend-duckdb/src/lib.rs`), same precedent as
  `fold_ledger_delta`/`execute_write_and_refresh_fingerprint_sidecar`.
- `FileStore::take_legacy_reconciliation_store()` (`crates/smelt-state/src/file_store.rs`)
  replaces `load_/save_reconciliation_store` — reads and deletes a legacy
  `.smelt/targets/<t>/reconciliation.json`, posture-ungated. `execute.rs` calls it once per run,
  before the first maintenance write, and imports both gradings into the engine
  (`_smelt_frontier` for `Frontier` entries, `_smelt_ledger` for `DeltaIdentities` entries) on a
  DuckDB target; on any other dialect the file is left untouched (no builder called at all).
- `execute.rs`'s per-batch region-recompute write now calls the new hook instead of
  load/mutate/save JSON; on a non-DuckDB backend it `tracing::warn!`s naming the model and skips.
- Spec deltas: `run_state.md` (layout, locking, atomic-writes prose, and the "Relationship to
  the reconciliation ledger" section rewritten to the engine-resident end state plus the
  legacy-import sentence), `incremental_models.md` (new residency paragraph under "The frontier
  record (reconciliation ledger)"; the DuckDB-only Known Divergences bullet now names both
  tables), `state.md` (the `.smelt/`-resident divergence bullet replaced with a narrower
  DuckDB-only-dialect bullet — the flagship gap is closed).

## Decisions

- **`write_group` is empty at the real call site.** The per-batch model write already commits
  earlier in the batch loop via `execute_model_incremental`/the column-scoped MERGE dispatch —
  by the time the frontier reset runs, there is no pending write to fuse into one transaction
  with it. The new hook still gives the reset's own delete+insert one atomic commit point; full
  fusion with the model's own write would require restructuring the per-batch write path to
  build and execute its own `StatementGroup` through this hook instead of
  `execute_model_incremental`, which is a materially larger, riskier change (it touches the
  tested `statement_parity`/`execute_parity` gates) than this phase's stated scope. Documented
  as a follow-up, not silently dropped.
- **Legacy import handles both gradings**, even though production never wrote `DeltaIdentities`
  to the JSON file after MP12 shipped (decision log, phase 4 plan) — a binary old enough to
  predate MP12 could have. Cheap to support, and the plan's own test list required it.
- **`Backend::execute_write_and_reset_frontier`'s default now skips `execute_statement_group`
  for an empty `write_group`** — found via `statement_parity.rs`'s spy-backend tests, which
  record every `execute_statement_group` call; an unconditional call was polluting the recorded
  groups with a spurious empty entry. Same fix belongs to any future default-trait caller passing
  an empty group, so it lives in the trait default, not just this call site.
- `.claude/hardening-baseline.txt`'s `smelt-backend-duckdb expect` count moved 18→19 via the
  gate's own `--update` — the new `execute_write_and_reset_frontier` override adds one more
  `connection.lock().expect("DuckDB connection mutex poisoned")`, the same infallible pattern
  every other transactional override in that file already uses.

## For the next planner

- **Not achieved: fusing the frontier reset with the model's own data write in one transaction.**
  The spec text says "commits in the same backend transaction as the recompute's own write";
  today's wiring gives the reset atomicity with itself, not with the write. Closing that gap
  cleanly means routing the DuckDB DeleteInsert batch write through
  `execute_write_and_reset_frontier`'s `write_group` instead of `execute_model_incremental`,
  which needs its own scoped phase (touches `statement_parity`, `execute_parity`, retry logic).
  Not in scope here; flagging so it isn't silently assumed done.
- Criterion 2 ("the reconciliation ledger is engine-resident … the additive grade's
  never-fold-twice check no longer rides on `.smelt/`") is now fully closed — confirmed the
  additive grade already didn't ride on `.smelt/` before this phase (phase 4 plan's own
  ground-truth finding), and the frontier grade now doesn't either.
- Phase 6 (state-deletion conformance leg) should now be able to assert deleting `.smelt/`
  mid-sequence never corrupts a region-recompute-only (idempotent-graded) model either, not just
  keyed additive folds — worth widening that leg's assertions when it lands.
- Phase 7's docs sweep should double check `state.md`'s first Known Divergences bullet ("The
  runtime ignores `state.mode` entirely") — it reads stale already (phases 2–3 wired posture
  gating), unrelated to this phase's scope but worth folding into that sweep.

## Gates

- `bash .claude/scripts/verify-phase.sh` (full): fmt ✓, clippy (zero warnings) ✓, `cargo test`
  (whole workspace) ✓, `example_diagnostics` ✓ — all green.
- `cargo test -p smelt-state --test reconciliation` ✓, `cargo test -p smelt-state --lib` ✓
- `cargo test -p smelt-backend-duckdb --lib` ✓
- `cargo test -p smelt-runtime --test frontier_residency --test state_posture --test statement_parity --test execute_parity --test keyed_reprocessed_window_refusal` ✓
- `cargo test -p smelt-cli --test maintenance_conformance` ✓ (70 tests, includes the retargeted
  `persisted_reconciliation_store_reflects_recompute_reset`)
- `cargo test -p smelt-logical --test walk_coverage` ✓
- `cargo test -p smelt-core --test hardening_budget` ✓ (baseline updated, see Decisions)
