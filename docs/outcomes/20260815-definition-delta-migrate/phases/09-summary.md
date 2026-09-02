# Phase 9 — summary

**Shipped:**
- `DeployedSchemaInput` Salsa world-fact input (`crates/smelt-db/src/lib.rs`): `model`,
  `project_root`, `columns`, `model_sql`, project-scoped like `LoaderFileInput`. `Database::
  set_deployed_schema`/`deployed_schema`, `Workspace.deployed_schemas`, and a
  `find_deployed_schema` lookup helper for Salsa-tracked queries.
- `smelt-db/src/workspace_ingest.rs::register_deployed_schemas_from_disk` reads every
  `.smelt/targets/<target>/schemas/<model>.json` via `smelt_state::FileStore` and registers one
  input per model; unreadable/missing is a silent no-op (loader-file precedent). Wired into
  `ingest_loaded_workspace`, the CLI's `init_db`, and the LSP's per-project `initialize` loop —
  workspace-loading-parity rule.
- `smelt_logical::maintenance::derive::skeleton_clause_changed` (new pure check, `ModelInputs.
  old_sql: Option<&str>`): diffs the deployed snapshot's `model_sql` against the model's current
  SQL via `backbuild::diff::definition_diff`; a `SkeletonDiff::Changed` clause pushes
  `Refusal::SkeletonClauseChanged { reason }`, mapped to the same `DiagnosticCode::
  MaintenanceSkeletonChanged` as the existing column-position `SkeletonChanged` refusal (one
  code, two refusal shapes — the phase-1 decision).
- `maintenance_plan`/`maintenance_plan_report` (`smelt-db/src/lib.rs`) now resolve the
  registered snapshot and thread `model_sql` into `derive_model_maintenance_plan[_with_edges]`,
  so the refusal reaches `file_diagnostics` (LSP) and `smelt explain <model>` (CLI) ahead of any
  run — no prior `smelt run` needed.
- Spec: `definition_deltas.md` §Detection (world-fact paragraph), `architecture.md` §Workspace
  loading parity rule (added to the eager-discovery list), `model_properties.md` and
  `incremental_models.md` Known Divergences bullets removed/trimmed.
- Tests: 6 in `smelt-db/tests/maintenance_diagnostics.rs`, 1 real-`Backend` LSP test in
  `smelt-lsp/tests/example_workspaces.rs`, 1 real-binary CLI test in
  `smelt-cli/tests/explain_definition_delta.rs`.

**Decisions:**
- `deployed_column_names` stays `&[]` in `maintenance_plan`/`maintenance_plan_report` — only
  `model_sql` is threaded from the new world fact. Feeding real column names there additionally
  derives a live `Trigger::ColumnAdded` cell in the pre-execution diagnostic gate, and that
  cell's own admission can refuse `MaintenanceScanUnbounded` for a column add that
  `smelt-runtime`'s narrower `resolve_live_in_place_update_cell` still executes safely (same
  full plan, but that caller only ever inspects the one cell it looks for). Discovered via two
  real e2e regressions (`schema_evolution_incremental`, `full_refresh_escape_rebuild`) — fixed
  by scoping the new world-fact wiring to the skeleton-clause check only, matching the phase's
  stated scope.
- `skeleton_clause_changed` treats `DefinitionDiff::Opaque` as no-signal (not a refusal) —
  matches every other `deployed_*`-gated derivation's "no positive proof, no refusal" posture.

**For the next planner:**
- `deployed_column_names` is still hardcoded `&[]` outside `smelt-runtime`'s maintenance driver —
  a live `ColumnAdded`/`InPlaceUpdate` diagnostic ahead of a run (vs. only the skeleton-changed
  half) is explicitly out of scope here and would need the pre-execution gate's admission
  posture reconciled with the runtime driver's narrower dispatch first.
- Phase 17 still owns the rest of the "Locality and diagnostic residues" bullet in
  `incremental_models.md` (column-group-scoped dirt, hour granularity, grain-alignment check
  scope) — only the surfacing clause was removed here.
- `smelt-runtime`/test call sites that already had real I/O access to the deployed schema
  (`maintenance_driver.rs`, `propagation.rs`) still pass `None` for `deployed_model_sql` (the
  skeleton-clause check) — they were out of this phase's scope; threading real `model_sql`
  through them would extend the check to the live run path too.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
- `cargo test -p smelt-db --test maintenance_diagnostics` — 15 passed
- `cargo test -p smelt-lsp --test diagnostics --test example_workspaces` — 37 passed
- `cargo test -p smelt-cli --test maintenance_conformance --features duckdb` — 74 passed
- `cargo test -p smelt-runtime --test execute_parity` — 4 passed
- `cargo test -p smelt-cli --test e2e --features duckdb` (regression check for the fix) — 175
  passed
