# Phase 12 summary — Per-cell frontier addressing / diff_patch over the region DeleteInsert default

## Shipped

- `smelt_logical::contract::deferral::cell_address` (`crates/smelt-logical/src/contract/deferral.rs`)
  — the stable, order-insensitive `{sorted columns}@{on}` cell identity, plus tests.
- `ModelIntervals.cell_frontiers: HashMap<String, String>` (`crates/smelt-state/src/intervals.rs`)
  with `record_cell_frontier`/`cell_frontier` accessors, `#[serde(default)]` for old-ledger
  compat, plus tests (`cell_frontiers_default_when_absent_from_an_old_ledger`,
  `recording_a_cell_frontier_advances_only_that_cell`).
- `ModelRunRecord.deferred_cells: Vec<String>` (`crates/smelt-state/src/lib.rs`) — schema
  landed, `skip_serializing_if = "Vec::is_empty"`.
- `smelt_runtime::contract_probes::deferral_cell_decisions` — pure per-cell licensing builder,
  independent of the model-level `deferral_decision`, plus two new tests in
  `contract_deferral_schedule.rs`.
- `choice.rs`: `ChosenTechnique::DiffPatch { recompute: DeleteInsert, .. }` now grants
  `DeleteLeg::Complete` (was `Omitted`) — a region recompute's own write scope IS its
  completeness argument. Updated the pre-existing test that asserted `Omitted` to assert
  `Complete` (this was exactly the divergence this phase closes).
- `resolve_live_membership_recompute_cell` (`maintenance_driver.rs`) now routes a
  `write: diff_patch` pin (previously silently `continue`d past) to a real
  `MembershipRecomputeWrite::DiffPatch` route, admitted via `admit_diff_patch` and executed via
  `execute_diff_patch` with a trivial `"TRUE"` slice predicate (the candidate is the model's own
  full unwindowed state, so nothing is excluded). New real-DuckDB e2e test in
  `technique_lowering.rs`:
  `diff_patch_pin_over_region_delete_insert_default_writes_only_the_difference` — RED before
  (manifest strategy fell through to `cumulative_aggregate`), GREEN after (`strategy ==
  "diff_patch"`, unchanged rows untouched across two redelivery runs).
- Spec updates: `incremental_models.md` §"`diff_patch`" states the region-completeness argument
  generally; both relevant Known Divergences bullets narrowed to reflect exactly what shipped.
  `run_state.md` documents `cell_frontiers` and `deferred_cells`.

## Decisions

- **Scoped diff_patch routing to `resolve_live_membership_recompute_cell` only**, per the plan's
  own framing ("the ONE path that reaches it"). The plain windowed/partition-grain region
  default (`resolve_incremental_strategy`) consults no cell-choice/write-pin logic at all and is
  untouched.
- **Did not wire per-cell deferral scheduling to any live dispatch site.** Discovered mid-phase:
  `contract.cells[].deferral` is validly declarable only on a clocked, interval-representable
  `on:` (`validate_deferral`'s own admission rule excludes `mutable_snapshot` and unclocked
  sources), but every currently-wired per-cell dispatch resolver
  (`resolve_live_membership_recompute_cell`, `resolve_live_column_scoped_cell`,
  `resolve_live_per_group_recompute_cell`'s `UpstreamMutation` branch) serves exactly such an
  inadmissible trigger. A cell that could validly declare `deferral` only exists on the ordinary
  `Trigger::NewData` fold cell over an append-only source — the plain
  windowed-incremental/cumulative-fold dispatch, a materially larger and riskier integration
  surface than this phase's remaining budget could responsibly cover. I built and tested the
  full pure/data-layer stack (address, ledger frontier, decision builder, manifest field) and
  then reverted a first attempt at wiring it into the membership resolver once I proved that
  wiring could never fire for a validly-declared cell — keeping dead-but-plausible-looking
  runtime plumbing would have been actively misleading.

## For the next planner

- **Real next step for per-cell deferral**: wire `deferral_cell_decisions` into the plain
  incremental batch loop / cumulative fold dispatch (wherever `Trigger::NewData{source}` per
  clocked source is actually dispatched today) — that is the only trigger family where
  `contract.cells[].deferral` can be validly declared. This is a materially bigger change than
  this phase (touches the model's primary write path, not a narrow UpstreamMutation resolver)
  and should get its own phase.
- The plan's test list named `crates/smelt-runtime/tests/repair_lowering.rs` for tests 8/9; I
  placed the real diff_patch-over-region test in `technique_lowering.rs` instead, next to the
  existing membership-recompute e2e fixture — that module already has the DuckDB harness and
  model shape the diff_patch route actually serves, and `repair_lowering.rs`'s own fixture
  (`raw.orders`, `PerGroupRecompute`) is a different technique family entirely.
- `ModelRunRecord.deferred_cells` and `ModelIntervals.cell_frontiers` are real, tested, and
  ready for a future dispatch site to populate — nothing currently writes a non-empty
  `deferred_cells`, which is honest today (spec says so) but should stop being true once the
  fold-dispatch wiring above lands.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings both feature
  sets, full `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-logical --test contract_deferral_cells --test diff_patch` — 13 passed.
- `cargo test -p smelt-runtime --test contract_deferral_schedule --test contract_deferral_probe --test repair_lowering --test technique_lowering --test statement_parity --test execute_parity` — all passed.
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb` — 74 passed.
- `cargo test -p smelt-db --test contract_deferral_diagnostics` — 4 passed.
