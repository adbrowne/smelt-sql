# Phase 10 summary — `smelt explain`: per-column guarantee ledger + refusal surfacing

**Shipped:**
- `crates/smelt-logical/src/maintenance/ledger.rs` (new): `SettleLabel` (widens
  `locality::SettleBound` with a `NotDerived` case), `ColumnGuarantee`
  (`Contract`/`DeterminismExemption`), `GuaranteeRow`, pure
  `derive_guarantee_ledger(column_groups, contract_cfg, key_locality, determinism)`,
  and pure `render_refusal(&Refusal) -> RefusalSummary`. 5 unit tests.
- `smelt-db`: `MaintenancePlanResult` gains `column_determinism: Vec<ColumnDeterminism>`,
  populated only in `maintenance_plan_report` from `model_property_vector`'s own walk
  (phase 9's `own_output_delta` precedent — every other construction site defaults empty).
- `crates/smelt-cli/src/explain.rs`: the pre-execution refusal block moved from the
  bottom of the report to immediately after the headline (before `Maintenance plan:`),
  now rendered via `ledger::render_refusal` instead of `{:?}`; a new model-level
  `Guarantees:` block prints after "Key temporal locality"/before `Relation contract:`.
  New `ExplainGuaranteeJson`/`ExplainRefusalJson` + `explain_guarantees_json`/
  `explain_refusals_json` expose the same fields on `--json`, wired into
  `build_maintenance_plan_json` (two new trailing params) and its one call site.
- Spec: `incremental_models.md` §Surface "CLI" splits the old "Per cell" bullet into
  **Refusals** (moved up), **Per cell** (contract/locality only now), and a new
  model-level **Guarantees** bullet; the determinism-scope Known Divergences bullet
  narrowed to its runtime-pinning half only; the "does not yet print the per-column
  guarantee summary" bullet deleted. `cli.md` gains **Refusals**/**Guarantees** prose
  paragraphs after "Effective contract".
- Docs-site `cli.md` prose + golden text block updated; `explain_show_sql_daily_events_golden.txt`
  regenerated; tutorial fixture (`deduplication.md`) regenerated via
  `python3 examples/web_analytics/generate_tutorial.py`.
- Tests: 5 `smelt-logical` unit, 1 `smelt-db` integration (`tests/maintenance_ledger.rs`),
  3 `smelt-cli` (`explain_prints_per_column_guarantee_ledger`,
  `explain_surfaces_refusals_before_the_plan`, `explain_json_guarantees_match_text`).

**Decisions:**
- A group's effective contract is resolved against its own first (lexicographically
  least) `mutation_sensitivity` source as the trigger address — the same convention
  `Trigger::NewData.source` uses (documented in `ledger.rs`'s doc comment). A group
  triggered by 2+ sources with differing per-cell `deferral` overrides under-reports
  (shows only the first trigger's resolution) rather than fabricating a merged value;
  `frozen_horizon` and a model-level `deferral` default are trigger-independent so this
  only affects a narrow multi-trigger + per-cell-override combination.
- `SettleBound` is honestly widened to `SettleLabel` with a `NotDerived` case rather than
  reusing `Option<SettleBound>` directly at call sites — keeps the "not derived" state a
  first-class render target instead of a `None`-check scattered across callers.
- Refusal code strings (`MaintenanceScanUnbounded`, `KeyedForbidsTimeseries`, etc.) are
  hardcoded per `Refusal` variant rather than round-tripped through `DiagnosticCode`,
  since 4 of the 7 `Refusal` variants (`ReachNotDerivable`, `RepairKeysNotDiscoverable`,
  `RepairSliceUnbounded`, plus `UnsupportedGrain`'s catalogue-row-precedes-variant case)
  have no `DiagnosticCode` enum variant yet per `docs/specs/diagnostics.md`'s own ledger.

**For the next planner:**
- Row 11 (walk fix for `group_by_output_keys`) is unaffected by this phase.
- Not addressed (correctly out of scope): wiring the determinism scope's *runtime* half
  (compile-time pinning removal, conformance-oracle/technique-gate consultation) — the
  spec bullet narrowed here says explicitly that stays open, tracked at
  `docs/research/20260816-open-questions-triage.md`.
- Discovered gap for row 13's close-out sweep: `Refusal::ReachNotDerivable`,
  `RepairKeysNotDiscoverable`, and `RepairSliceUnbounded` still have no `DiagnosticCode`
  enum variant (per `docs/specs/diagnostics.md`'s own note) — `render_refusal` names them
  by their documented catalogue string today, but a future `DiagnosticCode` addition
  should reuse that same string rather than drift.

**Gates:**
- `cargo test -p smelt-logical --test walk_coverage --quiet` — pass (4/4).
- `cargo test -p smelt-db --test maintenance_ledger --test maintenance_signature --quiet` — pass (1+3).
- `cargo test -p smelt-cli --test explain_maintenance --test explain_model --test explain_show_sql --quiet` — pass (29+27+6).
- `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md docs/specs/cli.md` — no matches (only the rule statement itself in `cli.md`'s own timeless-oracle banner).
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy, full workspace `cargo test`, `example_diagnostics`).
