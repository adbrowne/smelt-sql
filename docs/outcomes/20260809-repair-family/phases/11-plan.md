# Phase 11 plan — Surface: `smelt explain` rendering, docs-site update

## Objective

Make the repair family legible on the surface: `smelt explain <model>` renders a
`Technique::PerGroupRecompute` cell as a repair cell — the affected-key slice it recomputes by
(labelled a sound over-approximation), the bounded per-group read slice, how the affected keys are
discovered for the trigger's source posture, and — for a `write: diff_patch` pin — the resolved
write mechanism and its delete-leg verdict. The user docs gain the same story. Advances success
criterion 5, and closes criterion 6's docs half.

## Spec delta

`docs/specs/incremental_models.md` §Surface "CLI" (the `smelt explain <model>` bullet, ~line 336):
extend it with one sentence naming what a repair cell additionally prints — its **affected-key
slice** (with the over-approximation note), its **bounded per-group read slice**, its
**affected-key discovery mechanism** (clamped current-source scan, or the group-grain
fingerprint-sidecar diff for a `mutable_snapshot` source, §"The repair family" obligation 7), and
the resolved **write mechanism + delete-leg verdict** when a `diff_patch` pin matches the cell.
No behaviour outside the CLI report changes; §"The repair family" itself needs no edit.

## Tests

Red-green, in this order:

1. `smelt-logical`, `maintenance::repair` unit — `discovery_posture_is_sidecar_only_for_mutable_snapshot`:
   the new pure predicate returns `SidecarDiff` for `MutationProfile::MutableSnapshot` and
   `ClampedScan` for every other posture.
2. `smelt-runtime`, existing `repair_lowering.rs` — must stay green after the resolver is
   refactored to call the shared predicate (no new test; regression oracle for the refactor).
3. `smelt-cli/tests/explain_maintenance.rs::explain_renders_repair_cell_key_slice_and_read_bound` —
   stage a `RepairRecipe` (`render::stage_repair`, `RepairWriteMode::TargetedDeleteInsert`), assert
   the report names the technique, the affected-key columns, the over-approximation note, and the
   per-group read slice's `source`/`column`/bounds.
4. `…::explain_renders_repair_discovery_posture` — same fixture (a clocked `mutable_snapshot`
   source): the report names the group-grain fingerprint-sidecar diff as the discovery read.
5. `…::explain_renders_diff_patch_write_mechanism_and_delete_leg` — `RepairWriteMode::DiffPatch`
   recipe: the report names `diff_patch` as the resolved write mechanism with a **complete** delete
   leg.
6. `…::explain_non_repair_cell_prints_no_repair_stanza` — an existing keyed-fold fixture prints
   none of the new lines (the stanza is technique-scoped, not unconditional).

## Tasks

1. Spec first: land the §Surface "CLI" sentence above.
2. Add `RepairDiscoveryPosture` + `discovery_posture(mutation: MutationProfile)` to
   `crates/smelt-logical/src/maintenance/repair.rs` — the single owner of "which affected-key
   discovery read this source posture needs" (pure, doc-comment citing §"The repair family"
   obligation 7).
3. Refactor `maintenance_driver.rs`'s `resolve_live_per_group_recompute_cell` to branch on
   `repair::discovery_posture(facts.mutation)` instead of its inline
   `facts.mutation == MutationProfile::MutableSnapshot` comparison; its dialect gate, digest-column
   derivation and fail-loud `MaintenanceRepairDigestColumnsMissing` bail stay exactly where they are.
4. Thread the discovered `&[SourceInfo]` into `smelt_cli::explain::build_maintenance_plan_report`
   (already in scope at `commands/explain.rs`'s call site) and build the trigger source's facts via
   the existing `smelt_db::queries::maintenance::source_facts` — never a second mutation-profile
   mapping.
5. In `build_maintenance_plan_report`, emit a repair stanza for `cell.technique ==
   Technique::PerGroupRecompute` only: `repair key slice`, `repair read bound`, `affected-key
   discovery`. Read the key from `cell.row_identity.identity` (`RowIdentity::Key`) and the bound
   from the `cell.scans` entry matching the trigger's source — the same selection rule the runtime
   resolver uses; a cell missing either prints a named "not derived" line rather than a silent
   omission.
6. For a repair cell whose `matching_write_pin` resolves via `lookup_write_pattern`, call
   `choice::resolve_cell_choice` (with the overrides already computed in this function's ladder)
   and render `ChosenTechnique::DiffPatch`'s `DeleteLeg` verdict; a `ChoiceRefusal` surfaces as an
   `explain` error, matching the existing decidable-refusal precedent in this function.
7. docs-site: add a short "Repairing only the affected groups" section to
   `docs-site/docs/guide/incremental-models.md` (retraction over a mutable dimension → repair cell,
   the explain stanza verbatim, the `write: diff_patch` pin), and correct the `--technique`
   accepted-name lists in `docs-site/docs/reference/smelt-explain.md` and
   `docs-site/docs/reference/cli.md` to the exact set `parse_technique_arg` accepts (they currently
   omit `column_scoped_merge` and `per_group_recompute`).
8. Refresh any explain output quoted in `docs-site/docs/examples/web-analytics/` only if this
   phase's rendering actually changes it (non-repair cells must be byte-identical — test 6).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test explain_maintenance --test explain --test explain_show_sql`
- `cargo test -p smelt-runtime --test repair_lowering --test statement_parity`
- `cargo test -p smelt-logical --test walk_coverage`
- `cargo test -p smelt-cli --test maintenance_conformance`

## Commit message

`feat(incremental): smelt explain renders repair cells and the diff_patch write mechanism`
