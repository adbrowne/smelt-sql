# Phase 8 plan — the diagnostic rename lands in code, one code across both mechanisms

## Objective

Rename the diagnostic-code identity `MaintenanceSkeletonColumnAdded` → `MaintenanceSkeletonChanged`
everywhere it is user-visible (the `DiagnosticCode` variant, the `smelt-db` mapping and its message,
the `ledger.rs` refusal-code string `smelt explain` prints, the `diagnostics.md` catalogue row), and
extend it to `smelt migrate`'s own `SkeletonChange` verdict so the *one* code covers both
mechanisms. Sweep the sibling specs that still name the old code. Advances success criterion 6's
rename half (its surfacing half is phase 9).

## Spec delta (spec-first — the implement step makes these edits first)

- `docs/specs/diagnostics.md` — §catalogue: rename the `MaintenanceSkeletonColumnAdded` row to
  `MaintenanceSkeletonChanged`, widening its prose from "a field was added in a skeleton position"
  to "added or changed", and note that `smelt migrate`'s skeleton-change verdict names the same
  code. §Known Divergences: rename the mention in the "Five of the ten plan/graph `Maintenance*`
  codes" paragraph; leave its "only fires when the query has a real deployed-schema snapshot"
  sentence intact (phase 9 closes that).
- `docs/specs/definition_deltas.md` — §Known Divergences: delete the "**The diagnostic code is not
  yet renamed in the implementation**" bullet and replace it with a bullet stating only what
  survives: the code is not yet surfaced ahead of a run because `smelt-db`'s query has no
  deployed-schema input, tracked at phase 9.
- Sibling-spec sweep (rename only, no prose restructuring):
  `docs/specs/model_transforms.md` (5 occurrences), `docs/specs/model_properties.md` (§"Definition
  change column classification" + the not-yet-surfaced Known Divergence bullet),
  `docs/specs/incremental_models.md` (§Diagnostics cross-reference + the locality/diagnostic-residue
  Known Divergence bullet), `docs/specs/schema_evolution.md` (§pre-plan-behaviours paragraph).

## Tests

- `crates/smelt-db/tests/maintenance_diagnostics.rs::column_added_trigger_skeleton_position_refuses`
  — existing test; assert the mapped diagnostic code is `MaintenanceSkeletonChanged` (red until the
  variant is renamed).
- `crates/smelt-logical/src/maintenance/ledger.rs` unit test
  `skeleton_refusal_names_the_renamed_code` — `render_refusal` (the string `smelt explain` prints)
  emits `MaintenanceSkeletonChanged`.
- `crates/smelt-cli/tests/migrate.rs::skeleton_change_plan_names_the_diagnostic_code` — a plan whose
  group verdict is `SkeletonChange` prints `MaintenanceSkeletonChanged` in both the human render and
  the `--json` payload, so the one code spans both mechanisms.
- `crates/smelt-db/tests/integration/diagnostics_catalogue.rs::every_diagnostic_code_is_catalogued`
  — existing standing gate; it goes red on the enum rename until the catalogue row is renamed, and
  is the proof the sweep reached `diagnostics.md`.
- Repo-wide ratchet: `crates/smelt-db/tests/integration/diagnostics_catalogue.rs::
  no_old_skeleton_code_name_in_specs_or_code` — `MaintenanceSkeletonColumnAdded` appears nowhere in
  `crates/`, `docs/specs/`, or `docs-site/docs/` (mirrors phase 4's `no_backbuild_verb_in_user_docs`
  ratchet; `docs/plans/` and `docs/outcomes/` are historical and excluded).

## Tasks

1. Make the spec edits above (spec-first), including the catalogue row and the reworded
   `definition_deltas.md` divergence bullet.
2. Add the four new/updated tests; confirm each is red for the right reason.
3. Rename the `DiagnosticCode::MaintenanceSkeletonColumnAdded` variant in
   `crates/smelt-db/src/diagnostics_types.rs` and fix the one mapping site in
   `crates/smelt-db/src/lib.rs` (`file_diagnostics`), widening its message from "column '{c}' …
   never a column backfill" to name a skeleton *change* and cite `definition_deltas.md`
   §"Skeleton changes are a new relation" instead of the stale `incremental_models.md` §"The
   definition-change trigger" pointer.
4. Rename the `code:` string in `crates/smelt-logical/src/maintenance/ledger.rs`
   (`render_refusal`'s `Refusal::SkeletonColumnAdded` arm).
5. Name the code in `crates/smelt-cli/src/commands/migrate.rs`'s `Verdict::SkeletonChange` render
   (human label and the `--json` verdict payload — add a `diagnostic_code` field rather than
   changing the existing `"skeleton_change"` tag, so the JSON contract only grows).
6. Leave the internal pure variants (`smelt_logical::maintenance::Refusal::SkeletonColumnAdded`,
   `smelt_db::queries::maintenance::MaintenanceRefusal::SkeletonColumnAdded`) named as they are —
   they are not user-visible — but update each one's doc comment to name
   `MaintenanceSkeletonChanged` as the code it maps to.
7. Sweep remaining stale mentions in code doc comments (`smelt-db/src/queries/maintenance.rs`,
   `smelt-db/tests/maintenance_diagnostics.rs`) so the new ratchet passes.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-db --test maintenance_diagnostics --quiet`
- `cargo test -p smelt-db --test integration diagnostics_catalogue --quiet`
- `cargo test -p smelt-logical --lib maintenance::ledger --quiet`
- `cargo test -p smelt-cli --test migrate --features duckdb --quiet`

## Commit message

`refactor(diagnostics): rename MaintenanceSkeletonColumnAdded to MaintenanceSkeletonChanged`
