# Phase 14 summary — Per-cell `deferral` dispatch on the plain fold

**Shipped:**
- `smelt_logical::contract::deferral::{DeclaredFoldCell, FoldDeferralVerdict, fold_deferral_verdict}`
  (`crates/smelt-logical/src/contract/deferral.rs`) — the single-owner coverage rule: `SkipFold`
  only when every one of the fold's column groups is fully covered (union of matching declaring
  cells' columns) by cells that are ALL skip-licensed; partial coverage or any unlicensed covering
  cell always falls through to `Proceed`. 4 new unit tests.
- `maintenance_driver::resolve_fold_deferral` (`crates/smelt-runtime/src/maintenance_driver.rs`) —
  derives the model's maintenance plan (same shape as `resolve_incremental_strategy`), reads its
  `Trigger::NewData` creation source and `result.column_groups` as the fold's own groups, pairs
  `contract.cells[]` deferral declarations with caller-resolved `CellDeferralDecision`s, and calls
  `fold_deferral_verdict`. No independent lag comparison.
- `contract_probes::advance_cell_frontiers` — the write-side counterpart of
  `deferral_cell_decisions`'s read: advances exactly the named addresses' `cell_frontiers` entries,
  touching no sibling. 1 new unit test.
- `execute.rs` wiring: `deferral_declared` widened to include cell-level declarations;
  the pre-run snapshot pass now also resolves each model's fold-deferral verdict (skipping the
  cell-level check entirely when the model is already model-level-skipped) and records
  `deferral_skipped_cells`/`deferral_fold_addresses`; the skip manifest entry populates
  `deferred_cells` from the former; the incremental success path's interval-store critical section
  calls `advance_cell_frontiers` for the latter after a fold that actually ran. 2 new real-DuckDB
  `execute_project` e2e tests in `contract_deferral_skip_e2e.rs`.
- Spec: `docs/specs/incremental_models.md` §Known Divergences bullet rewritten (full-coverage-only
  rule, surviving residue restated precisely); §"The contract lattice" deferral paragraph gained
  one sentence on the coverage requirement. `docs/specs/run_state.md` §"Run manifest" dropped the
  "once a live dispatch site resolves…" qualifier on both `deferred_cells` and `cell_frontiers`.

**Decisions:**
- "Fold groups" = the model's own `result.column_groups` (from `derive_column_groups`) paired with
  the *first* `Trigger::NewData` cell's source — mirrors `resolve_incremental_strategy`'s existing
  `.find()` pattern (one creation cell drives the plain fold; `Grain::Partition` never derives more
  than one). A multi-source join's per-group provenance is still exactly `column_groups`; this
  phase does not attempt to split "which group came from which of several NewData sources" since
  no such cell exists today.
- `deferral_fold_addresses` (all declaring addresses for a model, independent of verdict) is a
  separate map from `deferral_skipped_cells` (only `SkipFold`'s addresses) — a `Proceed` run still
  needs to know which cells to advance, and the spec's "it runs, and its frontier advances with the
  rest" rule applies to every declaring cell, not just the ones that happened to license coverage.
- `ModelRunRecord.deferred_cells` on the SUCCESS path stays `Vec::new()` (unchanged) — the field's
  own doc comment already defines it as "empty ... when every declaring cell ran this cycle", which
  is exactly the success-path case.

**For the next planner:**
- The e2e fixture only exercises a single-source, single-column-group model (`total_amount` is the
  fold's only payload column). A model with multiple column groups and a cell covering only one of
  them (the "partial coverage — falls through to Proceed" case) is unit-tested in `smelt-logical`
  but not exercised end-to-end through a real `execute_project` run; worth adding if a future phase
  touches this dispatch again.
- Did not address the `group_by_unique_key`/`order_id` keyword-collision bug phase 13 flagged
  (still open, phase 17 on the outcome's own table).

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings both feature sets,
  full `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-logical --test walk_coverage` — 4/4 passed.
- `cargo test -p smelt-runtime --test contract_deferral_schedule --test contract_deferral_skip_e2e --test execute_parity --test statement_parity` — 8/4/4/23 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb` — 74/74 passed.
- No `smelt-generate` doc blocks drifted (`git status` clean on `docs-site/`/`examples/`).
