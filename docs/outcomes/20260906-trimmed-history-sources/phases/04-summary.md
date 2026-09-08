# Phase 04 summary — Refuse or degrade, never silent

**Shipped:**
- `crates/smelt-logical/src/maintenance/retention.rs` (new): `SourceRetentions`
  (bare source name → `DataLatency`), `RetentionDowngrade`, and the total
  `retention_outcomes(&HashMap<String, RetentionVerdict>) -> (Vec<Refusal>, Vec<RetentionDowngrade>)`.
- `Refusal::SourceRetentionExceeded` in `maintenance/refusal.rs`, mapped to the
  `SourceRetentionExceeded` diagnostic code.
- `MaintenancePlan.retention_downgrades: Vec<RetentionDowngrade>` (plan-level,
  like `fingerprint_projections`).
- `derive_maintenance_plan_with_referential_integrity_and_retentions` in
  `maintenance/derive/plan.rs` — folds `derive_retention_verdicts` +
  `retention_outcomes` onto the plan; the two existing entry points delegate
  with an empty retentions map.
- smelt-db: `build_source_retentions` (`plan_helpers.rs`), threaded through
  `derive_model_maintenance_plan`'s existing `source_refs` parameter (no new
  parameter added — avoids touching its dozens of call sites).
  `MaintenanceRefusal::SourceRetentionExceeded` + `RetentionDowngradeDiagnostic`
  + `MaintenancePlanDiagnostics.retention_downgrades`; `DiagnosticCode::{SourceRetentionExceeded,SourceRetentionDowngraded}`;
  `file_check.rs` surfaces the downgrade as a Warning.
- Spec: `sources.md` §Semantics 5 + diagnostic table, `model_properties.md`
  §"Reach versus retained history" (action half), `diagnostics.md` catalogue.
- Tests: `crates/smelt-logical/tests/retention_admission.rs` (5), two in
  `smelt-db/src/queries/maintenance/tests.rs`, one in
  `smelt-cli/tests/example_diagnostics/retention_diagnostics.rs` plus new
  `examples/broken/models/retention_exceeded.sql` +
  `sources/retention_exceeded_events.yml` fixtures.

**Decisions:** see outcome.md's 2026-09-09 (phase 4 planning) entries — `Exceeds`
refuses, `UnprovableWithin` downgrades, `Within`/`NoDeclaredBound` silent; the
downgrade follows the degradation contract's doctrine but not its
`StateDowngrade` record; retentions threaded as a side channel via the
existing `source_refs` parameter rather than a new one.

**For the next planner:** phase 5 (bound-moving-is-an-event) owns `window_age`
— `derive_maintenance_plan_with_referential_integrity_and_retentions` already
takes it (hardcoded `Seconds::ZERO` in `derive_maintenance_plan_impl`, doc
comment names phase 5). No other follow-up surfaced.

**Gates:** `bash .claude/scripts/verify-phase.sh` (ALL GREEN — fmt, clippy
both feature sets, full workspace `cargo test`, `example_diagnostics`);
`cargo test -p smelt-logical --test walk_coverage`;
`cargo test -p smelt-runtime --test statement_parity`;
`cargo test -p smelt-runtime --test execute_parity`;
`cargo test -p smelt-db --test integration diagnostics_catalogue`;
`cargo test -p smelt-cli --test example_diagnostics`;
`bash .claude/scripts/large-file-check.sh` — updated
(`diagnostics_types/mod.rs`, `file_check.rs`, `queries/maintenance/tests.rs`,
`smelt-lsp/src/backend/mod.rs` all grew from the required new diagnostic
variants/tests; sign-off: each delta is exactly the new code's own footprint).
