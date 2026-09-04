# Phase 6 plan — the recorded downgrade becomes visible surface

## Objective

Make the `state_downgrade` phase 4 recorded on `PlanCell` and phase 5 wired into the runtime
seam actually *visible*: `smelt explain <model>` prints it in the text report and in `--json`.
With a real user-visible channel in place, retire the last `RunReporter::state_structure_unavailable`
caller (the keyed-grain merge-ledger skip in `maintenance_driver.rs`) and delete the method.
Advances criterion 4 (the explain half), criterion 5 (`warehouse_tables: none` is observable),
and criterion 3/7 (the keyed-grain reporter event is replaced by the recorded downgrade).

## Spec delta

Implement step makes these edits first.

1. `docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan report" — document the per-cell
   `state downgrade:` text row (original technique, missing structure, reason) and the matching
   `state_downgrade` object (`original`, `missing`, `reason`) in each `--json` `cells[]` entry,
   omitted entirely when the cell was not downgraded (never rendered `null`, matching the
   existing `contract_point` posture). State that the report resolves availability offline from
   the model's declared target dialect and `state.warehouse_tables`, citing `state.md`
   §"The degradation contract".
2. `docs/specs/incremental_shapes.md` lines ~1251 and ~1396 — both name
   `RunReporter::state_structure_unavailable` as today's stand-in channel for the skipped
   merge-ledger record. Rewrite to the end state: the cell's recorded downgrade, printed by
   `smelt explain`. (Phase 5's summary flagged these as untouched; this phase makes them stale,
   so it owns them.)

## Tests

Red-green; each fails before its task lands.

- `crates/smelt-cli/tests/explain_maintenance.rs::explain_text_prints_state_downgrade` — a
  keyed-fold model whose declared target is a ledger-less dialect prints a `state downgrade:` row
  naming the original technique and the missing structure.
- `…::explain_json_carries_state_downgrade` — the same model's `--json` `cells[0].state_downgrade`
  has `original`, `missing`, `reason`.
- `…::explain_omits_state_downgrade_on_duckdb` — a DuckDB target prints no downgrade row and the
  JSON cell has no `state_downgrade` key at all.
- `…::warehouse_tables_none_downgrades_on_duckdb` — `state: { warehouse_tables: none }` produces
  the downgrade even on DuckDB (criterion 5's observable consequence).
- `crates/smelt-runtime/tests/keyed_frontier_bookkeeping.rs::keyed_ledger_skip_reports_no_reporter_event`
  — driving the keyed merge-ledger step on a non-DuckDB dialect records no reporter event; the
  cell's `state_downgrade` is the channel.
- `crates/smelt-runtime/tests/availability_seam.rs::no_state_structure_unavailable_reporter_remains`
  — structural: `crates/smelt-runtime/src` and `crates/smelt-cli/src` contain zero occurrences of
  `state_structure_unavailable` (comment lines included — the method is gone, not just unused).

## Tasks

1. Land the two spec edits above.
2. `crates/smelt-cli/src/commands/explain.rs`: derive the target's `SqlDialect` alongside the
   existing `MaintenanceDialect` derivation, build a `StateAvailability` via
   `smelt_runtime::maintenance_availability::availability_for_run(dialect, &config)`, and call
   `smelt_logical::maintenance::availability::resolve_availability(&mut result.plan.cells, &availability)`
   once, before the report/JSON/`--show-sql` paths consume `result` (so all three agree).
3. `crates/smelt-cli/src/explain.rs` `build_maintenance_plan_report`: emit a
   `      state downgrade: <original> → <technique> (missing: <structure>) — <reason>` row
   directly under the existing `technique:` row when `cell.state_downgrade.is_some()`.
4. Same file: add `ExplainStateDowngradeJson { original, missing, reason }` and an
   `Option<…>` `state_downgrade` field on `ExplainCellJson` with
   `#[serde(skip_serializing_if = "Option::is_none")]`; populate it in
   `build_maintenance_plan_json`.
5. `crates/smelt-runtime/src/maintenance_driver.rs` (~line 661): drop the
   `retry.reporter.state_structure_unavailable(...)` call in the ledger-less `else` branch,
   replacing it with a `tracing::debug!` that points at the cell's recorded `state_downgrade`
   (mirroring phase 5's `execute.rs` treatment) and updating the surrounding comment block, which
   currently asserts the reporter channel as the named-fact mechanism.
6. Delete `RunReporter::state_structure_unavailable` from `crates/smelt-runtime/src/reporter.rs`,
   its `CliReporter` impl in `crates/smelt-cli/src/reporter.rs`, the buffered `ReporterEvent`
   variant and replay arm in `crates/smelt-runtime/src/execute.rs` (~lines 250/342) with its
   `buffered_state_structure_unavailable_replays_to_reporter` test, and the three test-local
   impls (`execute.rs`, `maintenance_driver.rs`, `tests/statement_parity.rs`).
7. Update `statement_parity.rs`'s phase-5 test
   (`ledger_reset_is_skipped_on_a_non_duckdb_dialect`, ~line 6074) and its doc comment: it
   currently asserts the reporter method is *not called*; with the method gone it asserts the
   emitted-statement set only, and the comment cites `smelt explain` as the channel.

## Verification

- `bash .claude/scripts/verify-phase.sh` (mandatory)
- `cargo test -p smelt-cli --test explain_maintenance --test explain_model`
- `cargo test -p smelt-runtime --test availability_seam --test statement_parity --test execute_parity --test keyed_frontier_bookkeeping`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `rg -n 'state_structure_unavailable' crates/` returns nothing

## Commit message

`feat(state-residency): explain prints the recorded state downgrade; retire the reporter stand-in`
