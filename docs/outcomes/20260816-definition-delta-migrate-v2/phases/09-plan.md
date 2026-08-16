# Phase 9 — Surface the definition-change refusal ahead of a run

**Objective.** Give `smelt-db`'s analysis layer a real deployed-column set so
`MaintenanceSkeletonChanged` fires *before* a run: a project-scoped Salsa input carries the
deployed column names per model, populated once at the workspace-loading edge, and both
consumers (LSP `file_diagnostics`, `smelt explain`'s refusal block) read it. Closes the second
half of success criterion 6 (the rename half landed in phase 8) and removes the matching Known
Divergences bullets.

## Spec delta (made first, by the implement step)

- `docs/specs/definition_deltas.md` §Detection — add a short normative paragraph: the deployed
  column names recorded in the per-model schema snapshot are read at workspace load (once, at the
  edge — never inside an analysis query) and are the "before" side the pre-run skeleton-change
  diagnostic diffs against; a model with no snapshot derives no definition-change trigger
  (fail-closed, never a guess).
- `docs/specs/definition_deltas.md` §Known Divergences — delete the "The diagnostic code is not
  yet surfaced ahead of a run" bullet.
- `docs/specs/model_properties.md` §Known Divergences — delete the
  "`MaintenanceSkeletonChanged` is not yet surfaced as an LSP/CLI diagnostic ahead of a run"
  bullet.
- `docs/specs/diagnostics.md` §Known Divergences — rewrite the `MaintenanceSkeletonChanged`
  sentence inside the "Five of the ten plan/graph `Maintenance*` codes" paragraph: it now fires
  from the Salsa query against the deployed-schema input, so the code moves out of the
  "specified and unimplemented" list (adjust the sentence and count wording accordingly).
- `docs/specs/incremental_models.md` line ~2027 — update the reachability sentence to name the
  Salsa-query path as well as the maintenance driver.

## Tests (red-green)

1. `crates/smelt-db/tests/maintenance_diagnostics.rs::skeleton_change_fires_from_deployed_columns_input`
   — a project on disk whose `.smelt/targets/<t>/schemas/<model>.json` lacks a column the current
   SQL adds in a `GROUP BY` position; `file_diagnostics` yields
   `DiagnosticCode::MaintenanceSkeletonChanged`.
2. `…::deployed_columns_matching_current_sql_yield_no_diagnostic` — same fixture with the snapshot
   listing every current output column: no maintenance diagnostic (no false positive).
3. `…::missing_snapshot_yields_no_definition_change_trigger` — no `.smelt/` at all: today's
   behaviour, empty trigger set (fail-closed).
4. `crates/smelt-db/src/workspace_ingest.rs` unit test `ingest_populates_deployed_columns` — after
   `ingest_loaded_workspace`, the `ProjectInput`'s deployed-columns field carries the snapshot's
   model → column names for the effective target.
5. `crates/smelt-cli/tests/explain*` (extend the existing explain test target)
   `explain_refusal_block_names_skeleton_change` — `smelt explain <model>` over the same fixture
   prints the refusal naming `MaintenanceSkeletonChanged`.
6. `crates/smelt-lsp/tests/diagnostics.rs::skeleton_change_surfaces_through_the_lsp` — the real
   LSP backend publishes the diagnostic with code string `maintenance-skeleton-changed`.
7. `crates/smelt-lsp/tests/…::schema_snapshot_change_refreshes_diagnostics` — writing/updating a
   snapshot file and delivering it via `did_change_watched_files` clears (or raises) the
   diagnostic without a restart.

## Tasks

1. Apply the spec delta above.
2. Add `smelt-state` as a production dependency of `smelt-db` (deps are `smelt-core`/`-types`/
   `-dialect`, all already present; `smelt-state` does not depend on `smelt-db`, so no cycle).
3. Add a `deployed_columns` field to the `ProjectInput` Salsa input (`crates/smelt-db/src/lib.rs`)
   — a deterministically sorted `Vec<(String, Vec<String>)>` of model name → deployed column
   names, `#[returns(ref)]`; `set_project_input` creates it empty.
4. Add `Database::set_project_deployed_columns(&mut self, root: &Path, columns: …)`, mirroring
   `set_project_smelt_yml`, as the only mutation point.
5. Add the edge reader to `crates/smelt-db/src/workspace_ingest.rs`:
   `read_deployed_columns(project_root, target) -> Vec<(String, Vec<String>)>` over
   `smelt_state::FileStore`'s `list_deployed_model_names` + `load_schema`, and call it from
   `ingest_loaded_workspace` (step 4, after `set_active_target`) so CLI and LSP populate it
   identically — the workspace-loading-parity rule keeps this in exactly one place. A missing
   `.smelt/`, unreadable dir, or unparsable snapshot yields an empty entry, never a panic.
6. Thread the field into the two analysis call sites that today hardcode `&[]`:
   `maintenance_plan` (tracked query) and `maintenance_plan_report` in
   `crates/smelt-db/src/lib.rs` — look the model up by the same table name the query already
   derives from the file stem, pass its columns to
   `maintenance_plan_diagnostics` / `derive_model_maintenance_plan_with_edges`. Replace the
   "no I/O access (Salsa purity)" comments at both sites and in
   `queries/maintenance.rs::maintenance_plan_diagnostics` (which gains a
   `deployed_column_names` parameter — it stays a pure function; the input arrives from the
   caller).
7. CLI: where a `--target` override is applied after `init_db` (`commands/run.rs`,
   `build.rs`, `rebuild.rs`, `run_setup.rs`, `argument_resolution.rs` as applicable), re-populate
   via `set_project_deployed_columns` so the read target matches the active target; `explain`
   uses the ingest-time default.
8. LSP: extend `derive_watch_globs` with `.smelt/targets/*/schemas/*.json` and handle that path
   class in `did_change_watched_files` by re-reading the project's deployed columns and
   re-publishing diagnostics.
9. Update the phase-8-era doc comments that assert the path is unreachable
   (`crates/smelt-db/tests/maintenance_diagnostics.rs` around the `SkeletonColumnAdded` test,
   `queries/maintenance.rs`, `maintenance_driver.rs:815`) to describe the wired path.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-db --test maintenance_diagnostics --quiet`
- `cargo test -p smelt-lsp --test diagnostics --quiet` and
  `cargo test -p smelt-lsp --test example_workspaces --quiet`
- `cargo test -p smelt-cli --test explain --quiet` (or the explain test target's real name)
- `cargo test -p smelt-runtime --test execute_parity --quiet`

## Commit message

`feat(diagnostics): surface MaintenanceSkeletonChanged ahead of a run via a deployed-columns Salsa input`
