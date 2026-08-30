# Phase 9 — surface the definition-change diagnostic ahead of a run

## Objective

Close the second half of success criterion 6: `MaintenanceSkeletonChanged` today is reachable
only from `smelt-runtime`'s maintenance driver, because `smelt-db`'s diagnostics /
`smelt explain` path hands `derive_model_maintenance_plan` an empty `deployed_column_names`
and therefore derives no definition-change trigger at all. Add the deployed-schema snapshot as
a Salsa **world-fact input**, registered identically by the CLI's `init_db` and the LSP's
`initialize` (workspace-loading-parity rule), and thread it into both `maintenance_plan` (LSP
diagnostics) and `maintenance_plan_report` (`smelt explain`). The snapshot carries `model_sql`
as well as column names, so a skeleton *clause* change (a changed `GROUP BY`, a changed FROM
target) surfaces too — not only a skeleton-positioned column *add*.

## Spec delta

The spec edits land first (spec-first rule):

- `docs/specs/definition_deltas.md` §Detection — state that the deployed-schema snapshot is a
  world fact both the LSP and the CLI register at workspace load, so a pending skeleton change
  is reported as `MaintenanceSkeletonChanged` ahead of any run; a model with no snapshot on
  record derives no definition-change trigger (fail-closed, unchanged).
- `docs/specs/model_properties.md` §Known Divergences — remove the
  "`MaintenanceSkeletonChanged` is not yet surfaced as an LSP/CLI diagnostic ahead of a run"
  bullet (line ~363).
- `docs/specs/incremental_models.md` §Known Divergences, "Locality and diagnostic residues" —
  drop the surfacing clause and its "The surfacing gap is tracked: … phase 7b" sentence; the
  rest of that bullet stays open (phase 17 owns it).
- `docs/specs/architecture.md` §"Workspace loading parity rule (CLI ↔ LSP)" — add the
  deployed-schema registration to the list of inputs both consumers register at load.

## Tests

1. `smelt-db/tests/maintenance_diagnostics.rs::deployed_schema_input_surfaces_skeleton_changed`
   — a registered snapshot missing a newly added `GROUP BY` key makes `file_diagnostics` emit
   `MaintenanceSkeletonChanged`.
2. `…::no_deployed_schema_derives_no_definition_trigger` — with no snapshot registered the
   diagnostic set is byte-identical to today (fail-closed regression guard).
3. `…::deployed_schema_matching_current_definition_is_silent` — snapshot columns + SQL equal to
   the model on disk → no maintenance diagnostic.
4. `…::skeleton_clause_change_surfaces_without_a_column_add` — snapshot with the same column set
   but a different `GROUP BY` in `model_sql` → the refusal fires from the clause diff.
5. `…::updating_the_deployed_schema_input_reinvalidates` — re-setting the input flips the
   diagnostic on/off within one `Database` (Salsa invalidation is real, not load-time only).
6. `smelt-db/tests/maintenance_diagnostics.rs::register_deployed_schemas_from_disk_reads_target_schemas`
   — tempdir with `.smelt/targets/dev/schemas/<model>.json` registers one input per file; a
   missing/unreadable schemas dir is a silent no-op.
7. `smelt-lsp/tests/diagnostics.rs::lsp_publishes_skeleton_changed_from_deployed_schema` — the
   LSP's own initialize path (not a hand-built db) publishes wire code
   `"maintenance-skeleton-changed"` for the same fixture. This is the parity leg.
8. `smelt-cli/tests/` (alongside the existing explain tests)
   `::explain_reports_skeleton_change_from_deployed_schema` — `smelt explain <model>` names the
   refusal for a project whose `.smelt` snapshot predates the edit.

## Tasks

1. Spec edits above.
2. Add `smelt-state` as a `smelt-db` dependency (acyclic: `smelt-state` depends only on
   `smelt-types`/`smelt-dialect`).
3. `smelt-db/src/lib.rs`: `#[salsa::input] DeployedSchemaInput { model: Arc<str>, project_root:
   PathBuf, columns: Vec<Arc<str>>, model_sql: Option<Arc<str>> }`; `Database::set_deployed_schema`
   / `deployed_schema(project_root, model)` keyed registry plus a `deployed_schemas:
   Vec<DeployedSchemaInput>` field on the `Workspace` singleton — mirror `set_loader_file` /
   `loader_file` / `Workspace::loader_files` exactly, so tracked queries taking
   `&dyn salsa::Database` can enumerate without downcasting.
4. `smelt-db/src/workspace_ingest.rs`: `register_deployed_schemas_from_disk(&mut db,
   project_root, target)` reading via `smelt_state::FileStore::{list_deployed_model_names,
   load_schema}`; unreadable/unparseable entries are skipped with a `tracing::warn!`, the
   loader-file precedent (a stale snapshot must never fail workspace load).
5. Call it from `ingest_loaded_workspace` and from `smelt-cli`'s `init_db`, both with the same
   effective target expression (`smelt.yml` `target:` else `"dev"`), and from the LSP's
   `backend.rs` per-project ingest beside its `register_loader_files_from_disk` call. Where a
   CLI command carries its own `--target` override (`explain`), re-register with that target
   after `init_db` rather than duplicating the reader.
6. Thread the snapshot through: `maintenance_plan` and `maintenance_plan_report` look the model
   up by table name and pass its columns + `model_sql` to `maintenance_plan_diagnostics` (new
   params) → `derive_model_maintenance_plan`'s `deployed_column_names`, replacing the hardcoded
   `&[]` and its now-stale comment.
7. `smelt-logical`: add `old_sql: Option<&str>` to `ModelInputs`; in `derive.rs` add a pure
   skeleton-clause check calling `backbuild::definition_diff` and pushing a new
   `Refusal::SkeletonClauseChanged { reason }` on `SkeletonDiff::Changed`. Map it in
   `queries/maintenance.rs` to a sibling `MaintenanceRefusal` variant and in `lib.rs` to the
   **same** `DiagnosticCode::MaintenanceSkeletonChanged` (phase 1's "one code, not a split
   pair" decision — two refusal shapes, one code).
8. Re-run the hardening baseline updater if the `smelt-cli`/`smelt-db` counts move.

## Verification

- `bash .claude/scripts/verify-phase.sh` (includes `example_diagnostics` — watch it: any example
  workspace carrying a `.smelt` snapshot would now newly diagnose; none is committed today, so a
  failure here means a test fixture leaked a snapshot).
- `cargo test -p smelt-db --test maintenance_diagnostics`
- `cargo test -p smelt-lsp --test diagnostics --test example_workspaces`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `cargo test -p smelt-runtime --test execute_parity` (run-pipeline-parity rule)

## Commit message

`feat(db): surface definition-change refusals ahead of a run via a deployed-schema Salsa input`
