# Phase 25 — Reconcile the pre-execution gate's admission posture, thread real `deployed_column_names`

## Objective

`deployed_column_names` is still hardcoded `&[]` in every `smelt-db` call site (phase 9's
recorded residue) because feeding real names derives a live `Trigger::ColumnAdded` cell whose
own admission can refuse `MaintenanceScanUnbounded`/`MaintenanceNoAdmissibleTechnique` — an
Error ahead of a run for a column addition `smelt-runtime` executes safely (`execute.rs`
exempts a **pure column addition** from the definition-delta run gate outright). Give the
definition-change trigger's refusals their own provenance so the gate reports them at the
severity the runtime actually implies, then thread the real column names from the existing
`DeployedSchemaInput` world fact. Advances success criterion 6 (the definition-change
diagnostic surfaced ahead of a run — phase 9 closed only the skeleton-*clause* half) and
criterion 15's "not every refusal is `Error`".

## The posture rule (decided; do not re-litigate)

1. A refusal arising from `Trigger::ColumnAdded` because the add **cannot be backfilled in
   place** (unbounded scan for the column-scoped merge, no admissible technique, unresolvable
   expression, group disagreement) does **not** refuse the model's ongoing maintenance plan:
   the run proceeds, the column is ALTERed in, and its historical rows stay NULL until
   `smelt migrate`. Reported as a **Warning** naming the columns and `smelt migrate`.
2. A **skeleton-position** add keeps its existing `MaintenanceSkeletonChanged` **Error** —
   a grain change, per `definition_deltas.md` §"Skeleton changes are a new relation".
3. A model declaring `schema_evolution: strategy: full_refresh` derives **no** definition-change
   trigger in the gate at all (the runtime rebuilds the whole table; there is no in-place
   backfill obligation to report). Implemented as fact assembly in the `smelt-db` Salsa wrapper
   — pass `&[]` for that model — not as a new branch inside the pure derivation.

## Spec delta (lands first)

- `docs/specs/definition_deltas.md` §Detection, after the "Pure column addition is exempt"
  paragraph: state rules 1–3 above as the pre-execution gate's posture, explicitly tied to the
  run gate's own exemption (the gate never reports as an error what a run does not refuse).
- `docs/specs/diagnostics.md` §catalogue: new row
  `| MaintenanceColumnAddNotBackfillable | Warning | ... |`; trim the "Five of the ten plan/graph
  `Maintenance*` codes" bullet's clause claiming `MaintenanceSkeletonChanged` is the only
  deployed-snapshot-gated code now that the column-add half is wired too.

## Tests (red-green)

- `smelt-logical` (`maintenance/derive.rs` module tests or `tests/maintenance_skeleton.rs`):
  - `column_add_with_unbounded_merge_source_refuses_not_backfillable` — the unbounded
    column-scoped-merge path emits `Refusal::DefinitionChangeNotBackfillable`, not `ScanUnbounded`.
  - `column_add_not_proven_additive_refuses_not_backfillable` — the empty-sensitivity
    non-`PureBackfill` arms emit the same variant, not `NoAdmissibleTechnique`.
  - `skeleton_position_column_add_still_refuses_skeleton_changed` — rule 2 unchanged.
  - `admitted_pure_backfill_column_add_still_yields_in_place_update_cell` — no regression in the
    admitting path.
- `crates/smelt-db/tests/maintenance_diagnostics.rs`:
  - `not_backfillable_column_add_is_a_warning_naming_smelt_migrate` — Warning severity, new code.
  - `ongoing_fold_refusal_is_still_an_error_with_a_deployed_snapshot` — a genuine `NewData`
    `ScanUnbounded` keeps Error once real column names are threaded.
  - `full_refresh_schema_evolution_model_derives_no_definition_change_refusal` — rule 3.
  - `maintenance_plan_derives_the_column_added_cell_from_the_registered_snapshot` — the Salsa
    path (`maintenance_plan`/`maintenance_plan_report`) now sees a real `Trigger::ColumnAdded`
    cell, i.e. the threading actually happened.
- `crates/smelt-cli/tests/explain_definition_delta.rs`:
  `explain_reports_a_non_backfillable_column_add_as_a_warning` — `smelt explain` surfaces it
  without failing the command.
- Non-regression (must stay green, not new): `crates/smelt-cli/tests/e2e/` —
  `schema_evolution_incremental`, `full_refresh_escape_rebuild`;
  `crates/smelt-runtime/tests/schema_migration_backfill_atomicity.rs`;
  `crates/smelt-db/tests/integration` `diagnostics_catalogue` coverage.

## Tasks

1. Spec delta above (both files) — first.
2. Add `Refusal::DefinitionChangeNotBackfillable { columns: Vec<String>, why: String }` to
   `crates/smelt-logical/src/maintenance/mod.rs`; convert the four non-skeleton refusal pushes in
   `derive_column_added` (`derive.rs` ~1798–1880) to it; fix every exhaustive `match` (testkit
   `verdict.rs`/`render.rs`, `smelt-db` mapping) the compiler names.
3. Mirror it as `MaintenanceRefusal::DefinitionChangeNotBackfillable` in
   `crates/smelt-db/src/queries/maintenance.rs`'s refusal mapping.
4. Add `DiagnosticCode::MaintenanceColumnAddNotBackfillable` (`diagnostics_types.rs`); in
   `smelt-db/src/lib.rs`'s refusal→diagnostic loop, emit it at **Warning** while every other
   refusal stays Error (the loop currently hardcodes `DiagnosticSeverity::Error` — make severity
   part of the per-refusal match arm, not a constant).
5. In `smelt-db/src/lib.rs`'s two call sites (~1801 `maintenance_plan`, ~1968
   `maintenance_plan_report`), replace `&[]` with the registered snapshot's column names, gated
   by rule 3 (`metadata.schema_evolution.strategy == FullRefresh` ⇒ `&[]`); replace the two stale
   rationale comments with a pointer to the spec's posture paragraph.
6. Thread the same value through `maintenance_plan_diagnostics`' `deployed_column_names`
   parameter at its Salsa call site.
7. Run the non-regression list; if a fixture newly warns, confirm the warning is correct rather
   than silencing it.
8. `docs-site/docs/guide/backbuild-synthesis.md`: one sentence that a non-backfillable column add
   shows up as an editor/`explain` warning ahead of the run, and `smelt migrate` is the fix.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --lib maintenance` and `cargo test -p smelt-logical --test maintenance_skeleton`
- `cargo test -p smelt-db --test maintenance_diagnostics --test integration`
- `cargo test -p smelt-cli --features duckdb --test e2e --test explain_definition_delta`
- `cargo test -p smelt-runtime --test schema_migration_backfill_atomicity`
- `cargo test -p smelt-cli --test example_diagnostics`

## Commit message

`feat(maintenance): report a non-backfillable column add as a warning and thread the real deployed schema`
