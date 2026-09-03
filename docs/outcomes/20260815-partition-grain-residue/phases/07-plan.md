# Phase 7 plan — `partition_column` rename: refusal diagnostic + fixture

## Objective

Close the last implementation residue (audit bullet #7, success criterion 7): renaming a
maintained model's declared `timeseries.partition_column` between runs is accepted silently
today, because the deployed-schema snapshot records only the compiled SQL and the output column
names — never the declared write address. Persist the declared `partition_column` in the
snapshot, derive a pure refusal when it changes, and surface it as a named diagnostic with an
end-to-end fixture (refusal + `--full-refresh` remedy).

## Why a new code rather than `MaintenanceSkeletonChanged`

A rename that also renames the output column can *incidentally* trip the existing
`ColumnAdded` → skeleton-position path, naming the new column with no mention of the address
change; a rename that repoints `partition_column` at an already-projected column produces no
column diff at all and is entirely invisible. The address every partition-grain maintenance
write targets is its own world fact, so it gets its own snapshot field and its own code.
Scope: `partition_column` only — an `event_time_column`-only rename is not folded in (no
residue bullet asks for it; record it in the summary if the work makes it trivial).

## Spec delta (spec-first; the implement step makes these edits)

1. `docs/specs/incremental_shapes.md` §"The partition grain" — new short subsection (or an
   entry in that section's diagnostics list) stating: the declared `partition_column` is
   recorded in the deployed-schema snapshot; changing it on a maintained model whose table
   already exists is refused with `MaintenancePartitionColumnChanged` (Error), naming both the
   recorded and the current column; the remedy is `--full-refresh` (a rebuild re-addresses the
   table) or `smelt migrate`. Fail-closed: a snapshot with no recorded `partition_column`
   (written before the field existed, or no snapshot at all) derives no refusal.
2. Same file §Known Divergences → the "Schema evolution on the partition grain…" bullet loses
   its residual-`partition_column`-rename clause (the rest of the bullet, deferring output
   schema change to `definition_deltas.md`, stays).
3. `docs/specs/diagnostics.md` §catalogue — one row for `MaintenancePartitionColumnChanged`
   (Error), owner `incremental_shapes.md` §"The partition grain".
4. `docs-site/docs/guide/incremental-models.md` — a few lines under the partition-grain
   material: what happens when you rename `partition_column`, and the remedy.

## Tests (red-green, in this order)

1. `smelt-logical` `maintenance::derive` unit — `partition_column_rename_derives_refusal`:
   `ModelInputs` with `old_partition_col: Some("event_date")` and a `Grain::Partition {
   partition_col: "event_day" }` output yields `Refusal::PartitionColumnChanged { from, to }`.
2. `smelt-logical` — `unchanged_partition_column_derives_no_refusal` (case-insensitive equal
   names derive nothing).
3. `smelt-logical` — `absent_old_partition_column_fails_closed` (`None` derives nothing).
4. `smelt-state` — `deployed_schema_json_without_partition_column_deserializes`: a snapshot
   JSON predating the field round-trips with `partition_column: None`.
5. `smelt-db` `tests/maintenance_diagnostics.rs` —
   `renamed_partition_column_emits_maintenance_partition_column_changed`: a registered
   `DeployedSchemaInput` recording `event_date` against a model now declaring `event_day`
   emits the code; the sibling assertion with a matching name emits none.
6. `smelt-cli` `tests/partition_residue_probes.rs` —
   `probe_partition_column_rename_refusal` **inverted**: after a v1 `smelt run`, the renamed
   v2 fails `smelt check` with a message naming `partition_column` and both column names;
   a follow-up `smelt run --full-refresh` succeeds and a subsequent `smelt check` is clean
   (the remedy leg — proves the snapshot is re-recorded, not a dead end).

## Tasks

1. `smelt-state::schema_tracking::DeployedSchema` — add `#[serde(default)] pub
   partition_column: Option<String>` (same back-compat posture as `model_sql`).
2. `smelt_runtime::schema_evolution::save_deployed_schema` — new `partition_column:
   Option<&str>` parameter; fill it at both `execute.rs` call sites from the plan's model
   metadata (`timeseries.partition_column`), and at
   `smelt-maintenance-testkit/src/migrate_step.rs`.
3. `smelt-db` — `DeployedSchemaInput` gains `partition_column: Option<Arc<str>>`;
   `Database::set_deployed_schema` and `workspace_ingest::register_deployed_schemas_from_disk`
   thread it (mirror `model_sql` exactly, including the change-detection compare in
   `set_deployed_schema`).
4. `smelt-logical` — `ModelInputs::old_partition_col: Option<&'a str>`; a pure
   `partition_column_changed(&ModelInputs) -> Option<(String, String)>` beside
   `skeleton_clause_changed`, pushing `Refusal::PartitionColumnChanged { from, to }` (compare
   ASCII-case-insensitively against `output_partition_col()`; `None` → no refusal).
5. `smelt-db::queries::maintenance` — new `deployed_partition_column` parameter on
   `derive_model_maintenance_plan`, forwarded into `ModelInputs`; map the refusal to a new
   `MaintenanceRefusal::PartitionColumnChanged`; resolve the value at both
   `smelt-db/src/lib.rs` call sites (beside `deployed_model_sql`) and pass it (or `None` where
   no snapshot is resolved) at the six `maintenance_driver.rs` call sites.
6. `smelt-db` — `DiagnosticCode::MaintenancePartitionColumnChanged` variant + fold in
   `check_file_diagnostics`'s maintenance arm (message names both columns and the remedy).
7. Spec + catalogue + docs-site edits per §Spec delta; invert the probe per test 6.
8. Sweep `examples/` for any partition-grain model whose declared `partition_column` differs
   from a committed snapshot (expected: none — `.smelt/` is not committed); record the sweep.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt + clippy both feature sets + full test + example
  diagnostics).
- `cargo test -p smelt-cli --test partition_residue_probes --features duckdb`
- `cargo test -p smelt-db --test maintenance_diagnostics`
- `cargo test -p smelt-db --test integration diagnostics_catalogue`
- `cargo test -p smelt-runtime --test statement_parity`
- `cargo test -p smelt-cli --test maintenance_conformance`
- Write `phases/07-summary.md` (shipped / decisions / for the next planner / gates).

## Commit message

`feat(maintenance): refuse a partition_column rename with a named diagnostic`
