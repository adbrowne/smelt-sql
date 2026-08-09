# Phase 7 summary — runtime routing for the `diff_patch` write pin

## Shipped

- `emit_diff_patch` (`crates/smelt-logical/src/maintenance/emit.rs`) now takes one caller-composed
  `slice_predicate: &str` instead of a `(partition_col, Region)` pair — both delete legs restrict
  to it verbatim. `Region::predicate` made `pub` for a future region-partitioned caller.
- `resolve_repair_write` (new, pure, `crates/smelt-runtime/src/maintenance_driver.rs`): the
  decision table from a resolved `ChosenTechnique` to a `RepairWrite` — `TargetedDeleteInsert`,
  `DiffPatch { compared_columns, delete_leg }` (admitted via `diff_patch::admit_diff_patch`), or
  `Ok(None)`/`Err` for the rest. Split out of `resolve_live_per_group_recompute_cell`'s loop so
  it is independently unit-testable.
- `resolve_live_per_group_recompute_cell` now returns a 5-tuple (`LiveRepairCell`) including the
  resolved `RepairWrite`.
- `diff_patch_staged_relation`, `repair_slice_predicate` (the affected-key `EXISTS` predicate),
  and `execute_diff_patch` (emitter → retry → `execute_statement_group`), all in
  `maintenance_driver.rs`.
- `execute.rs`: the keyed window-forward branch now dispatches on the resolved `RepairWrite`;
  strategy label `diff_patch` when routed.
- Spec: `incremental_models.md` gained one sentence on `diff_patch`'s slice shape and the Known
  Divergences entry narrowed to the still-unrouted `DeleteInsert`-recompute case.

## Decisions

- `resolve_cell_choice`'s `DiffPatch { recompute }` always equals the loop's own pre-filtered
  `cell.technique` (`PerGroupRecompute`) inside `resolve_live_per_group_recompute_cell` — the
  "unroutable" bail arm is real fail-loud code but is not reachable via any production call path
  today (no caller passes `cell: None` or a `DeleteInsert`-technique cell into this resolver).
  Kept as defensive code per fail-loud discipline; unit-tested directly via `resolve_repair_write`
  rather than through a (impossible) full-integration repro.
- `repair_slice_predicate`'s table argument must be the **unqualified** table name (matching
  `execute_diff_patch`'s internal `DELETE FROM {table} WHERE ...` unqualified column refs), not
  the schema-qualified `full_table` — caught by the statement-parity byte-diff.

## For the next planner

- The spec's Known Divergences entry now states plainly: a `diff_patch` pin over the region
  `DeleteInsert` default is *unenforced*, not refused, because nothing currently calls
  `resolve_cell_choice` for that case with a write pin present. If phase 8/9 (or a future outcome)
  wants that case to actually refuse, it needs a resolver change, not just wiring — flagged, not
  fixed here (out of this phase's scope per the plan).
- `smelt explain` rendering of a diff-patch write and the conformance recipe are still phase 8/9's
  job, unchanged from the plan.

## Gates

- `bash .claude/scripts/verify-phase.sh` — fmt/clippy/full workspace `cargo test`/
  `example_diagnostics` all green.
- `cargo test -p smelt-logical --test diff_patch --test walk_coverage` — 11 + 4 passed.
- `cargo test -p smelt-runtime --test repair_lowering --test statement_parity --test technique_lowering --test diagnostics` — 10 + 21 + 10 + 27 passed.
- `cargo test -p smelt-cli --test maintenance_conformance --test explain` — 4 + 53 passed.
