# Phase 25 summary

## Shipped

- New `Refusal::DefinitionChangeNotBackfillable { columns, why }`
  (`crates/smelt-logical/src/maintenance/mod.rs`), replacing the four
  non-skeleton refusal pushes inside `derive_column_added`
  (`crates/smelt-logical/src/maintenance/derive.rs`): the three
  empty-mutation-sensitivity arms (disagreement, unresolvable expression,
  `UpstreamRederive` with no scannable source) and the column-scoped-merge
  `ScanUnbounded` arm.
- `MaintenanceRefusal::DefinitionChangeNotBackfillable` mirror in
  `crates/smelt-db/src/queries/maintenance.rs`, and
  `DiagnosticCode::MaintenanceColumnAddNotBackfillable` in
  `diagnostics_types.rs`, mapped to **Warning** severity (every other
  refusal stays Error) in `smelt-db/src/lib.rs`'s refusal→diagnostic loop —
  severity is now part of the per-arm match, not a hardcoded constant.
- Both `maintenance_plan` and `maintenance_plan_report` Salsa call sites in
  `smelt-db/src/lib.rs` now thread the real `DeployedSchemaInput` column
  names instead of `&[]`, gated to `&[]` when
  `schema_evolution.strategy == FullRefresh` (rule 3 — no obligation to
  report ahead of a run that will full-refresh anyway).
- Spec: `docs/specs/definition_deltas.md` §Detection gained the three-rule
  posture paragraph; `docs/specs/diagnostics.md` gained the
  `MaintenanceColumnAddNotBackfillable` catalogue row and updated the
  deployed-snapshot-gated-codes clause.
- Docs: `docs-site/docs/guide/backbuild-synthesis.md` gained a paragraph on
  the pre-execution warning.
- Tests: 4 new `smelt-logical` unit tests
  (`maintenance_column_add_not_backfillable.rs`), 4 new `smelt-db`
  integration tests (`maintenance_diagnostics.rs`, incl. a new
  `plan_for_in` helper), 1 new `smelt-cli` e2e test
  (`explain_definition_delta.rs`).

## Decisions

- Kept the "unknown source" `NoAdmissibleTechnique` arm inside the
  column-scoped-merge branch unconverted — that is a real configuration
  bug (a `mutation_sensitivity` name with no matching `SourceFacts`), not a
  "cannot backfill in place" shape.
- `full_refresh_schema_evolution` gating implemented identically at both
  Salsa call sites as fact assembly, per the plan's explicit instruction
  not to add a new branch inside the pure derivation.

## For the next planner

- Reclassifying two existing refusals broke two non-regression unit tests
  outside this phase's own file list
  (`maintenance_tracer.rs::ex36_without_the_additive_only_proof_fails_closed`,
  `maintenance_tracer_evolution.rs::v4_without_the_explicit_partition_predicate_refuses_scan_unbounded`)
  — both asserted the old `NoAdmissibleTechnique`/`ScanUnbounded` variant
  for exactly the column-add shape this phase retargets. Fixed in place;
  worth a note for future refusal-taxonomy changes that the full workspace
  suite (not just the plan's own listed tests) needs a pass before
  declaring green.
- No further follow-up identified beyond what rows 26+ already own.

## Gates

- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both
  feature sets, full workspace `cargo test`, example_diagnostics)
- `cargo test -p smelt-logical --lib maintenance --test maintenance_skeleton` — pass
- `cargo test -p smelt-db --test maintenance_diagnostics --test integration` — pass
- `cargo test -p smelt-cli --features duckdb --test e2e --test explain_definition_delta` — pass
- `cargo test -p smelt-runtime --test schema_migration_backfill_atomicity` — pass
- `cargo test -p smelt-cli --test example_diagnostics` — pass
- `cargo test -p smelt-lsp --test example_workspaces` — pass (extra, per CLAUDE.md)
- `cargo test --workspace` — pass (full sweep after fixing the two regressions above)
