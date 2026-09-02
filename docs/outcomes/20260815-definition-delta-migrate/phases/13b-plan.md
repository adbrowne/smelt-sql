# Phase 13b plan — Per-cell `deferral` dispatch on the plain fold

## Objective

Make `contract.cells[].deferral` a live scheduling decision instead of a derived-but-unread
fact: the plain `Trigger::NewData` incremental fold consults `deferral_cell_decisions`, declines
the fold when every column group it serves is licensed to skip, records the skipped cells in
`RunManifest`'s `deferred_cells`, and advances `IntervalStore::cell_frontiers` for the declaring
cells when it does run. Advances success criterion 9 (standing gates stay green over a newly
live lattice-point dispatch) and closes the remaining half of the per-cell-deferral divergence
that phases 12–13 narrowed.

## Spec delta (first)

- `docs/specs/incremental_models.md` §Known Divergences — rewrite the "Per-cell `deferral`
  scheduling has … no live dispatch site consults it yet" bullet: the plain windowed-incremental
  fold now resolves per-cell decisions, skips when every `Trigger::NewData` group it serves is
  covered by a skip-licensed declaring cell, and advances each declaring cell's frontier
  otherwise. State the surviving residue precisely: a declaring cell addressing a **strict
  subset** of the fold's column groups cannot decline work, because the plain fold's write is
  whole-row — it runs, and its frontier advances with the rest.
- `docs/specs/incremental_models.md` §"The contract lattice", deferral paragraph — one sentence
  stating that a per-cell skip requires full coverage of the fold's groups (partial coverage
  falls through to the normal path, as `lag ≤ 0` already does): skipping is a licensed
  relaxation, never a way to decline unlicensed work.
- `docs/specs/run_state.md` §"Run manifest" — drop the trailing "present … once a live dispatch
  site resolves per-cell decisions (§Known Divergences)" qualifier on `deferred_cells`.

## Tests

Red-green, in this order:

1. `smelt-logical` unit (`contract/deferral.rs`) `fold_deferral_verdict_skips_only_on_full_coverage`
   — every fold group matched by a skip-licensed declared cell ⇒ `SkipFold` naming the addresses.
2. `smelt-logical` unit `fold_deferral_verdict_proceeds_on_partial_coverage` — one fold group
   unmatched ⇒ `Proceed`, even though the matched cell is skip-licensed.
3. `smelt-logical` unit `fold_deferral_verdict_proceeds_when_a_covered_cell_is_not_licensed` —
   full coverage but one decision is `RunLicense::Proceed` ⇒ `Proceed`.
4. `smelt-logical` unit `fold_deferral_verdict_is_proceed_with_no_declared_cells` — the
   undeclared model never gets a per-cell skip.
5. `smelt-runtime` `contract_deferral_schedule.rs`
   `cell_frontier_advance_is_scoped_to_the_declaring_cells` — the frontier-advance helper touches
   exactly the resolved declaring addresses and no sibling entry.
6. `smelt-runtime` `contract_deferral_skip_e2e.rs`
   `per_cell_deferral_skips_the_fold_and_records_the_cell_address` — real `execute_project`:
   fixture model declares `contract.cells[]` with `deferral` covering the whole row on the
   clocked `on:` source, lag within `D` ⇒ manifest entry `skipped_deferral`, `deferred_cells`
   holds the cell address, target table row count unchanged, no interval written.
7. `smelt-runtime` `contract_deferral_skip_e2e.rs`
   `a_run_past_the_cell_window_folds_and_advances_the_cell_frontier` — lag beyond `D` ⇒ the fold
   runs, `intervals.json`'s `cell_frontiers[address]` equals the run's window end, and
   `deferred_cells` is empty on the success record.

## Tasks

1. Spec edits above (spec-first).
2. `crates/smelt-logical/src/contract/deferral.rs`: add `FoldDeferralVerdict { Proceed,
   SkipFold { addresses: Vec<String> } }` and the pure `fold_deferral_verdict(declared, fold_groups)`
   — single-owner coverage rule, no runtime types, keyed on `(sorted group columns, source)`.
3. `crates/smelt-runtime/src/maintenance_driver.rs`: `resolve_fold_deferral(...)` — derive the
   model's maintenance plan (same construction the sibling `resolve_live_*` resolvers use),
   list its `Trigger::NewData` cells as `(group columns, source)`, match each declaring
   `contract.cells[]` entry by group membership (`columns` names *any* member), pair with its
   `CellDeferralDecision`, and return `(FoldDeferralVerdict, Vec<String> /* declaring addresses */)`.
   No independent lag comparison here — the licenses come from `deferral_cell_decisions`.
4. `crates/smelt-runtime/src/execute.rs` pre-run deferral pass (~line 1255): widen
   `deferral_declared` to models declaring `contract.cells[].deferral`; for those, call
   `resolve_fold_deferral` with the same one-shot `interval_store`/`landed_deltas` snapshot;
   a `SkipFold` inserts into `deferral_own_skip` (so `propagate_deferral_skip` covers dependents)
   and records its addresses in a new `deferral_skipped_cells: HashMap<String, Vec<String>>`.
5. `execute.rs` skip manifest entry (~line 1495): populate `deferred_cells` from that map (empty
   for an upstream-propagated skip).
6. `execute.rs` incremental success path (~line 3730, inside the existing `state_io_lock`
   interval-store critical section): for each resolved declaring address, call
   `intervals.record_cell_frontier(address, &end_str)` — carried through from the pre-run pass,
   not re-derived per model.
7. Re-run `python3 examples/web_analytics/generate_tutorial.py` if any `smelt-generate` block drifts.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-runtime --test contract_deferral_schedule --test contract_deferral_skip_e2e
  --test execute_parity --test statement_parity`
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb`

## Commit message

`feat(contract): dispatch per-cell deferral on the plain incremental fold and record cell frontiers`
