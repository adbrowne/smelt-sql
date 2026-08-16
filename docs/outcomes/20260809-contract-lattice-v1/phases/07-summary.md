# Phase 7 summary — Surface: `explain` contract rendering + docs-site

## Shipped

- `smelt_logical::contract::{EffectiveContract, EffectiveDeferral, DeferralOrigin,
  effective_contract, matching_contract_cell}` in `crates/smelt-logical/src/contract/mod.rs` —
  the single-owner per-cell resolution of the effective contract lattice point: model-level
  `frozen_horizon` reaches every cell; `deferral` applies a `contract.cells[]` match (narrower
  wins) else the model-level default, mirroring `maintenance::choice`'s own addressing semantics.
  `EffectiveContract::render_label()` is the shared one-line description both renderings use.
- `crates/smelt-cli/src/explain.rs`: `build_maintenance_plan_report` gained a `contract_cfg`
  parameter and prints a `contract:  <label>` row per cell (text report), threading through the
  new `cell_trigger_address` helper. `build_maintenance_plan_json` gained `column_groups` +
  `contract_cfg` parameters and a `contract_point: ExplainContractPointJson` field on
  `ExplainCellJson` (`frozen_horizon`/`deferral`/`deferral_origin`, each
  `skip_serializing_if = "Option::is_none"` — a default cell's `contract_point` serializes as
  `{}`, never `null`-filled keys).
- `crates/smelt-cli/src/commands/explain.rs` reads `model.metadata.contract` and threads it to
  both builders.
- Tests: 4 new unit tests in `contract/mod.rs`, 3 new integration tests in
  `explain_maintenance.rs` (default/frozen_horizon text rows, `--json` `contract_point`), 1 new
  structural test in `contract_lattice_spec.rs` (`explain_contract_rendering_is_single_owned`).
- Spec: `docs/specs/cli.md` §"`smelt explain <model>` maintenance-plan report" gained the
  "Effective contract" paragraph; `docs/specs/incremental_models.md` §Known Divergences'
  contract-lattice bullet lost its "still missing" clause and now states the rendering is landed.
  `docs-site/docs/guide/incremental-models.md` gained a `## Contract relaxations` section;
  `docs-site/docs/reference/cli.md` gained one line on the `contract:` row / `contract_point`.

## Decisions

- Per-cell effective-contract resolution is single-owned in `smelt-logical`, never a local ladder
  in `smelt-cli` — enforced structurally by the new `contract_lattice_spec.rs` test (see 2026-08-10
  decision log entry on outcome.md).
- `build_maintenance_plan_json` needed a new `column_groups` parameter (it previously had no path
  from a `PlanCell`'s display-name `group` string back to real column names) — added rather than
  re-deriving column membership inside the JSON builder.

## For the next planner

- Two generated/golden fixtures needed regeneration as a direct, expected consequence of the new
  `contract:  default` row: `crates/smelt-cli/tests/fixtures/explain_show_sql_daily_events_golden.txt`
  (regenerated via a real `smelt explain --show-sql` run) and
  `docs-site/docs/examples/web-analytics/deduplication.md` (regenerated via
  `python3 examples/web_analytics/generate_tutorial.py`) — both additive two-line diffs, no other
  drift. Nothing left undone for this outcome's success criteria; this was the last row.
- This was the outcome's final planned phase (row 7 of 7) — all six success criteria are now met.
  The outcome can be marked `done` once this phase's row flips.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full `cargo test`,
  `example_diagnostics`).
- `cargo test -p smelt-logical --test contract_lattice_spec` — 13 passed.
- `cargo test -p smelt-cli --test explain_maintenance --test explain_show_sql --test
  explain_model` — 20 + 26 + 6 passed.
- `cargo test -p smelt-cli --test example_diagnostics` — 119 passed, 1 ignored (pre-existing).
