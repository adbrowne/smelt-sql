## Drift Report: incremental_models

**Spec**: docs/specs/incremental_models.md (last_reviewed: 2026-09-03)
**Date**: 2026-09-04

### Automated checks

- cargo fmt — PASS (already run once for this outcome; not re-run per instructions)
- cargo clippy — PASS (already run once for this outcome; not re-run per instructions)
- cargo test — PASS (already run once for this outcome; not re-run per instructions)
- example_diagnostics — PASS (already run once for this outcome; not re-run per instructions)

This report performed its own targeted verification on top of that green baseline: existence
checks for every Surface/CLI item, every path in §References → Code, every path in
§References → Tests, every docs-site page cited, and the diagnostic-code catalogue
cross-referenced against `crates/smelt-db/src/diagnostics_types.rs` and
`docs/specs/diagnostics.md`.

### Surface drift

- ✅ `refresh: incremental`, `timeseries:`, `unique_key:`, `grain:` (check-only) — declared shape
  surface confirmed in `models.md`/`timeseries.md` machinery and exercised throughout
  `crates/smelt-logical/src/maintenance/`.
- ✅ `maintenance:` block (`defaults.prefer`, `cells[].on/prefer/technique/write`, `scan_bounds`,
  `horizon_ceiling`) — pins and ladder resolution present in `crates/smelt-logical/src/maintenance/mod.rs`,
  `choice.rs`, exercised by `crates/smelt-cli/tests/maintenance_pins.rs`.
- ✅ `contract:` block (`frozen_horizon`, `deferral`, `retain_departed`, per-cell `deferral`) —
  triples present in `crates/smelt-logical/src/contract/{frozen_horizon,deferral,retain_departed}.rs`.
- ✅ CLI — `--since-upstream`/`--source`/`--landed` (`crates/smelt-cli/src/main.rs:201`),
  `--include-upstreams` (`:418`), `smelt explain --show-sql` (`:498`).
- ✅ docs-site pages cited in §References all exist: `index.md`,
  `guide/{incremental-models,sql-models,materializations}.md`, `concepts/how-it-works.md`,
  `reference/{timeseries,smelt-yml,cumulative-aggregate,cli}.md`.
- ✅ Diagnostics table (shared plan codes + contract-lattice codes) — every code the spec lists
  as owned here (`MaintenanceNoAdmissibleTechnique`, `MaintenanceScanUnbounded`,
  `MaintenanceWriteAddressingRefused`, `MaintenanceWritePatternUnavailable`,
  `ContractFrozenHorizonInvalid`, `ContractDeferralInvalid`, `ContractRetainDepartedInvalid`)
  has a matching `DiagnosticCode` variant. `MaintenanceReachNotDerivable`,
  `MaintenanceUnboundedFootprint`, `MaintenanceGraphUnsupportedNode`,
  `MaintenanceRepairKeysNotDiscoverable`, `MaintenanceRepairSliceUnbounded` have no
  `DiagnosticCode` variant yet (they fire as `anyhow`/error-string refusals, asserted by
  string-containment tests, not as structured LSP diagnostics) — this is documented and
  landing-planned in `docs/specs/diagnostics.md` line 558, so it is not undocumented drift;
  noted here for completeness only, no action needed.
- ✅ A new diagnostic found in code, `MaintenancePartitionColumnChanged` (added 2026-09-04,
  commit `6bb11ffc`), is correctly *not* in this spec's Diagnostics table — it is explicitly
  owned by `incremental_shapes.md` §"The partition grain" per its own doc comment and
  `docs/specs/diagnostics.md:523`. No drift.

### Semantics drift

- ✅ The equivalence invariant, order/set-determinacy corollary — `crates/smelt-cli/tests/maintenance_conformance`
  (generative oracle), `crates/smelt-maintenance-testkit` (s_tracker/oracle.rs).
- ✅ Delta signatures (derived, never declared; widen-never-narrow) —
  `crates/smelt-logical/tests/output_delta_spec.rs`, `crates/smelt-logical/src/analysis/output_delta.rs`.
- ✅ The algebraic maintenance ladder (rungs 1-4) — `crates/smelt-logical/tests/maintenance_choice.rs`,
  `maintenance/choice.rs`.
- ✅ The contract lattice (frozen_horizon/deferral/retain_departed triples; single ownership in
  `smelt-logical`) — `crates/smelt-logical/tests/contract_lattice_spec.rs`, `src/contract/`.
- ✅ The graph layer (forward propagation / backward resolution, adjointness) —
  `crates/smelt-logical/tests/maintenance_propagation_adjoint.rs`,
  `crates/smelt-runtime/tests/since_upstream_propagation.rs`, `tests/include_upstreams.rs`.
- ✅ Statement emission single ownership — `cargo test -p smelt-runtime --test statement_parity`
  exists at `crates/smelt-runtime/tests/statement_parity.rs`.
- ✅ Standing generative conformance gate — `crates/smelt-cli/tests/maintenance_conformance/gate.rs`
  present, plus BigQuery/Spark variants.
- ✅ Coverage-matrix inventory gate — `crates/smelt-logical/tests/maintenance_plan_conformance.rs`
  present.

No rule in §Semantics was found without at least one corresponding test file from
§References → Tests; all cited test paths exist on disk.

### Invariant drift

- ✅ Layered single ownership (`smelt-db` has no production dependency on `smelt-planner`;
  plan is pure data derived in `smelt-logical`) — consistent with the plan/derive/emit module
  layout in `crates/smelt-logical/src/maintenance/`; this is also asserted by the project-wide
  structural gate documented in `CLAUDE.md`, not re-derived here.
- ✅ Contract-lattice point single ownership (declaration schema + pure oracle transform + probe
  emitter, one triple per point, all in `smelt-logical`) — `src/contract/{frozen_horizon,deferral,retain_departed}.rs`
  each contain all three pieces.
- ✅ Fail-loud/fail-closed — every diagnostic code in the shared table maps to a named refusal;
  spot-checked `MaintenanceGraphUnsupportedNode`, `MaintenanceUnboundedFootprint` call sites in
  `crates/smelt-runtime/src/propagation.rs` and `maintenance_driver.rs` — all `bail!`/refuse,
  none silently fall back.
- ⚠️ "The plan is pure data, derived by pure functions, in one place; consumers never re-derive
  it" — verified by structure/module boundaries and citation of `architecture.md`, but a full
  cross-crate call-graph audit for re-derivation was out of scope for this pass; treat as
  spot-checked, not exhaustively proven.

### Timeless-oracle drift

- ✅ No phase-vocabulary leakage detected in the spec body: `rg -n "Phase [A-Z0-9]+" docs/specs/incremental_models.md`
  returns zero matches (checked after correcting an initial shell quoting error in the first
  attempt).

### Freshness

- last_reviewed: 2026-09-03
- most recent commit touching a §References → Code path: 2026-09-04T06:40:57+10:00,
  commit `6bb11ffc` ("refuse a partition_column rename with a named diagnostic"), touching
  `crates/smelt-logical/src/maintenance/{mod,derive}.rs` and `crates/smelt-runtime/src/{propagation,maintenance_driver}.rs`.
- That commit's entire substance (`MaintenancePartitionColumnChanged`) is explicitly owned by
  `incremental_shapes.md` §"The partition grain" (confirmed above), not by this spec's own
  normative content. No other §References → Code path changed after `last_reviewed`.
- Verdict: **fresh** — the one post-review commit touching shared files is substantively a
  sibling spec's change; nothing in this spec's own Surface/Semantics/Constraints needs
  updating on account of it.

### Summary

- Drift items: 0 requiring a fix, 0 requiring a phase row, 0 blocked. Everything checked in
  Surface, Semantics, Invariant, Timeless-oracle, and Freshness passed or was already
  correctly attributed to a sibling spec / already tracked in `docs/specs/diagnostics.md`.
- No inline fixes were needed or made — the spec's Diagnostics table, Surface section, and
  References are all internally consistent with the code and docs-site as inspected.
- Known Divergences / Open Questions items were cross-checked against
  `docs/outcomes/20260815-incremental-spec-closure-confirm/baseline-inventory.md` (IM-01
  through IM-25) and are not re-litigated here; all are already correctly classified as
  `open`, `closed <sha>`, or `drifted`+`accurate` by that independent audit.
- Recommended next step: none. No `/smelt:spec` or `/smelt:plan` action indicated by this pass.
