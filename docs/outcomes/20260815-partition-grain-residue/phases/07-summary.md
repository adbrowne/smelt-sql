# Phase 7 summary — `partition_column` rename: refusal diagnostic + fixture

**Shipped:**
- `DeployedSchema::partition_column: Option<String>` (`smelt-state/src/schema_tracking.rs`), same
  `#[serde(default)]` back-compat posture as `model_sql`; threaded through `DeployedSchemaInput`
  (`smelt-db/src/lib.rs`), `save_deployed_schema` (`smelt-runtime/src/schema_evolution.rs`), and
  `register_deployed_schemas_from_disk` (`smelt-db/src/workspace_ingest.rs`).
- `ModelInputs::old_partition_col` + the pure `partition_column_changed` derivation and
  `Refusal::PartitionColumnChanged` in `smelt-logical/src/maintenance/{derive,mod}.rs`
  (ASCII-case-insensitive compare against `output_partition_col()`, fail-closed on `None`).
- `MaintenanceRefusal::PartitionColumnChanged` → `DiagnosticCode::MaintenancePartitionColumnChanged`
  (Error) mapping in `smelt-db/src/lib.rs`, catalogued in `docs/specs/diagnostics.md`.
- Both `derive_model_maintenance_plan`/`_with_edges` and `maintenance_plan_diagnostics` gained a
  `deployed_partition_column: Option<&str>` parameter, resolved in `smelt-db/src/lib.rs` beside
  `deployed_model_sql`; every `smelt-runtime` call site (maintenance_driver.rs ×9,
  propagation.rs) passes `None` (none of those routes read a snapshot at all, matching the
  existing `deployed_model_sql: None` posture there).
- Tests: 3 unit tests in `smelt-logical` (`partition_column_changed_tests`), 2 in `smelt-state`
  (roundtrip + back-compat deserialize), 1 in `smelt-db/tests/maintenance_diagnostics.rs`
  (rename fires, matching name doesn't), 1 inverted end-to-end probe in
  `smelt-cli/tests/partition_residue_probes.rs` (refusal names both columns; deleting the stale
  snapshot and re-running clears it and records the new column).

**Decisions:**
- **The remedy is NOT `--full-refresh`.** The plan text (and my own first-drafted spec/diagnostic
  wording) claimed `--full-refresh` clears the refusal. Empirically verified false: the
  pre-execution analyzer gate (`smelt-runtime::gate::gate_diagnostics`, `architecture.md`
  §"Diagnostic parity rule") blocks on ANY Error-severity diagnostic unconditionally — "not a
  code allow-list" is explicit in its own doc comment — and runs before any run-flag branching, so
  `--full-refresh`/`--allow-full-refresh` cannot bypass it. Corrected the diagnostic message, the
  new spec constraint (rule 13), the diagnostics catalogue row, and the docs-site page to state
  the real remedy: delete the model's recorded snapshot
  (`.smelt/targets/<target>/schemas/<model>.json`) and re-run — fail-closed (`old_partition_col:
  None` derives no refusal) makes this work. `smelt migrate` was also verified NOT to help for a
  pure-frontmatter rename (SQL identical → "eclipsed — nothing to do" → `--apply` refuses with "no
  admissible in-place technique").
- **Fixture isolates the rename from `MaintenanceSkeletonChanged`.** A naive fixture (rename the
  output column too) tripped the pre-existing skeleton-change trigger instead, since the new
  column name was absent from the deployed column set. Redesigned per the plan's own rationale:
  two columns already both projected and grouped on in v1; v2 repoints `partition_column` at the
  sibling column with byte-identical SQL — no column diff, no skeleton diff, isolating
  `MaintenancePartitionColumnChanged` as the only fired refusal.
- **First-run save path reads `plan.model_file.metadata`, not `plan.incremental`.** A brand-new
  `refresh: incremental` model's very first run executes through `execute.rs`'s full-refresh save
  branch (no existing table yet), where `plan.incremental` is `None` even though the model
  declares `timeseries:`. Reading `partition_column` from `plan.incremental.as_ref().map(...)`
  there silently recorded `None` forever (verified by hand against a real run before fixing) — the
  first-deployment snapshot never carried a `partition_column`, so no future rename could ever be
  detected. Fixed to read `plan.model_file.metadata.timeseries.partition_column` directly, which
  is correct regardless of which branch executed this run.
- Scope held to `partition_column` only per the plan; an `event_time_column`-only rename was not
  investigated (no residue bullet asked for it).

**For the next planner:**
- The diagnostic's live-blocking, no-bypass nature (only remedy: delete the snapshot file) is a
  sharper edge than the original Known Divergences bullet implied. If a future outcome wants a
  smoother remedy (e.g. `smelt migrate --apply` updating just the recorded `partition_column`
  without a full plan-derivation attempt, or a `--force` flag on the gate), that is new scope, not
  something this phase's Tasks list asked for — `migrate.rs`'s `apply_plan` was updated to thread
  `partition_column` through `save_deployed_schema` but it PRESERVES the old value (no rename
  detection inside `migrate`'s own derivation), since migrate's `derive_plan` has no concept of a
  partition-column-only change at all.
- `docs/specs/incremental_shapes.md` §"The partition grain" Known Divergences bullet #3 (item
  correcting the "specified ahead of a tracking plan" claim, noted in the phase-6 decision log)
  is still open for phase 8's close-out — this phase's own edit only removed the
  `partition_column`-rename clause from bullet #3, it did not touch the stale-tracking-plan
  correction.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace test, example_diagnostics).
- `cargo test -p smelt-cli --test partition_residue_probes --features duckdb` — 3 passed.
- `cargo test -p smelt-db --test maintenance_diagnostics` — 31 passed.
- `cargo test -p smelt-db --test integration diagnostics_catalogue` — 1 passed.
- `cargo test -p smelt-runtime --test statement_parity` — 33 passed.
- `cargo test -p smelt-cli --test maintenance_conformance` — 75 passed.
- Had to update `.claude/unknown-census.toml` line numbers for 8 pre-existing `smelt-state`
  allowlist entries that shifted when the new `partition_column` field/tests were inserted
  (mechanical fmt-driven shift, not new `Unknown` sites).
