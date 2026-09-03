# Phase 7 plan — rename the skeleton diagnostic in code and sweep the sibling specs

## Objective

Land phase 1's decided rename end to end in the implementation: `MaintenanceSkeletonColumnAdded`
becomes `MaintenanceSkeletonChanged`, from the `smelt-logical` refusal variant through the
`smelt-db` mapping to the LSP code string, and every sibling spec that names the old code is
swept. Advances success criterion 6's rename half and criterion 8's sibling-spec discipline; the
"surface it ahead of a run" half moves to the new row 7b (see Decision log).

## Spec delta

The target names are already normative in `docs/specs/definition_deltas.md` (§Diagnostics table,
§Design "The skeleton-change diagnostic is one code"). This phase's spec work is the **sweep**, not
a new decision:

- `docs/specs/diagnostics.md` — catalogue row (line ~517) and the "five of the ten plan/graph
  `Maintenance*` codes" paragraph (~549) rename to `MaintenanceSkeletonChanged`. Required for the
  catalogue gate, which greps the spec for each enum variant name.
- `docs/specs/incremental_models.md` (~541, ~1865), `docs/specs/model_properties.md` (~252, ~363),
  `docs/specs/model_transforms.md` (57, 58, 231, 401, 503), `docs/specs/schema_evolution.md` (~145)
  — rename each mention. The *substance* of the two "not yet surfaced ahead of a run" divergence
  bullets (`model_properties.md` ~363, `incremental_models.md` ~1865) stays true and stays; only
  the code name inside them changes, and their tracker pointer retargets to phase 7b.
- `docs/specs/definition_deltas.md` §Known Divergences — the "**The diagnostic code is not yet
  renamed in the implementation**" bullet is **removed** (that is what this phase does) and replaced
  by nothing; the surfacing gap is already owned by the two bullets above.
- `docs/plans/` and `docs/handoffs/` are historical records and are **not** edited.

## Tests

1. `crates/smelt-db/tests/integration/diagnostics_catalogue.rs::every_diagnostic_code_is_catalogued`
   — existing gate; goes red on the enum rename until `diagnostics.md` is swept. Red-green driver
   for the spec edit.
2. `crates/smelt-db/tests/maintenance_diagnostics.rs` (existing skeleton test, ~425–471) — asserts
   `Refusal::SkeletonChanged` and the `DiagnosticCode::MaintenanceSkeletonChanged` mapping under
   their new names.
3. `crates/smelt-lsp/src/backend.rs` mapping — new unit test
   `skeleton_changed_maps_to_stable_code_string`: `DbCode::MaintenanceSkeletonChanged` renders as
   `"maintenance-skeleton-changed"` (the wire-visible string a CI/editor consumer matches on).
4. `crates/smelt-logical/tests/maintenance_tracer.rs` (~569, ~727) and
   `maintenance_tracer_evolution.rs` (~468) — the EX-39 refusal assertions compile and pass against
   `Refusal::SkeletonChanged`.
5. `crates/smelt-cli/tests/maintenance_conformance/gate.rs` (~4501) — the GROUP-BY-widening recipe
   still refuses under the new variant name.
6. New `crates/smelt-db/tests/maintenance_diagnostics.rs::no_stale_skeleton_column_added_spelling`
   — greps `crates/` and `docs/specs/` for `SkeletonColumnAdded`, asserting zero matches, so a
   half-done rename cannot pass green. (Excludes `docs/plans/`, `docs/handoffs/`, `docs/research/`,
   `docs/outcomes/`, `target/`.)

## Tasks

1. Rename `Refusal::SkeletonColumnAdded` → `Refusal::SkeletonChanged` in
   `crates/smelt-logical/src/maintenance/mod.rs` (~358) and its three push sites in
   `maintenance/derive.rs` (~1569, ~1580, ~1636); update doc comments to say "added or changed".
2. Rename `MaintenanceRefusal::SkeletonColumnAdded` → `SkeletonChanged` in
   `crates/smelt-db/src/queries/maintenance.rs` (~836, ~1150) and update the surrounding comments.
3. Rename `DiagnosticCode::MaintenanceSkeletonColumnAdded` → `MaintenanceSkeletonChanged`
   (`crates/smelt-db/src/diagnostics_types.rs` ~887) and its mapping arm in
   `crates/smelt-db/src/lib.rs` (~2526); the `MetadataError`/refusal matches stay exhaustive.
4. Rename the LSP code string in `crates/smelt-lsp/src/backend.rs` (~466) to
   `"maintenance-skeleton-changed"`; add test 3.
5. Update the four test files in tests 2/4/5 to the new names; add test 6.
6. Sweep the six sibling specs and remove the `definition_deltas.md` divergence bullet per §Spec
   delta; retarget the two surfacing bullets' tracker to phase 7b.
7. `cargo fmt --all`, then run the verification gates.

## Verification

- `bash .claude/scripts/verify-phase.sh` (fmt + clippy both feature sets + full `cargo test` +
  `example_diagnostics`).
- `cargo test -p smelt-db --test integration diagnostics_catalogue`
- `cargo test -p smelt-db --test maintenance_diagnostics`
- `cargo test -p smelt-logical --test maintenance_tracer --test maintenance_tracer_evolution`
- `cargo test -p smelt-cli --test maintenance_conformance`
- `rg -n 'SkeletonColumnAdded' crates/ docs/specs/` — no matches.

## Commit message

`refactor(diagnostics): rename MaintenanceSkeletonColumnAdded to MaintenanceSkeletonChanged`
