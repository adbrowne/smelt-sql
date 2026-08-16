# Phase 5 summary — refusal narrowing: the plan derives a repair cell

## Shipped

- `derive_new_data`'s key-grain posture-failure branch (`crates/smelt-logical/src/maintenance/
  derive.rs`) now attempts `repair::admit_per_group_recompute` before refusing: on `Ok`, pushes a
  `PerGroupRecompute` cell instead of the posture refusal; on `Err`, pushes the pre-existing
  `NoAdmissibleTechnique` refusal AND the mapped `Refusal::RepairKeysNotDiscoverable`/
  `RepairSliceUnbounded` — additive, never a replacement.
- `repair::delta_shape_for_source(sql, facts) -> DeltaShape` (`maintenance/repair.rs`): reuses
  `analysis::fingerprint::fingerprint_projection`'s walk-backed leaf classifier to build the
  delta's carried columns; fails closed to an empty column set on `Projection::FullRow`.
- `derive_repair_cell` now takes the actual `Trigger` being derived instead of hard-coding
  `Trigger::UpstreamMutation`.
- `choice::resolve_cell_choice`'s `DiffPatch` arm threads the slice-completeness premise:
  `Technique::PerGroupRecompute` discharges it (`DeleteLeg::Complete`); every other recompute
  keeps `DeleteLeg::Omitted { why }`.
- `docs/specs/incremental_models.md` §Known Divergences narrowed: derivation is done, runtime
  lowering and the executed-vs-emitted parity leg remain (phase 6).
- New test file `crates/smelt-logical/tests/repair_wiring.rs` (5 tests, all listed tests 1–5) plus
  2 unit tests for `delta_shape_for_source` (repair.rs) and 2 for the `DiffPatch` delete-leg
  (choice.rs) — 9 new tests total.

## Decisions

- Hook site confirmed: exactly `derive_new_data`'s faithful-fold source-posture failure branch
  (the retraction case), per the plan — no other refusal site was touched.
- `delta_shape_for_source` reuses `fingerprint_projection` directly (not a parameterised copy of
  the affected-keys leaf classifier) since it needs the unfiltered "every column this SQL reads
  from this source" set, which is exactly what `fingerprint_projection`'s `Projection::Columns`
  already computes.
- `DeleteLeg::Complete` (not a separate `Present` variant) is the existing enum's "sound and
  included" arm — the plan's phase-4 prose used "Present" informally.

## For the next planner

- Five pre-existing tests (`input_delta_consumed.rs`, `maintenance_coverage_matrix.rs` x2,
  `maintenance_plan_admission.rs` x2, `maintenance_tracer.rs`) asserted refusal-count == 1 for a
  `MutableSnapshot` source with no declared `unique_key` (a fixture pattern used throughout the
  crate); all now assert the additive `RepairKeysNotDiscoverable` refusal is present too — text
  assertions on the `NoAdmissibleTechnique` reason were preserved verbatim. No golden/conformance
  fallout beyond these — `maintenance_conformance`/`explain`/`explain_model`/`statement_parity`/
  `technique_lowering` all passed unchanged (no runtime lowering exists yet, so no admitted repair
  cell reaches those paths).
- Phase 6 (runtime lowering) needs: a `Technique::PerGroupRecompute` lowering arm (currently a
  fail-loud gap per phase 3's summary), the executed-vs-emitted `statement_parity` leg for both
  repair and `diff_patch`, and — per this phase's choice.rs comment — `DeleteInsert`'s own
  slice-completeness proof if `diff_patch`'s delete leg should ever cover the region-recompute
  case too (out of this phase's scope, not a success criterion).

## Gates

- `cargo test -p smelt-logical --test repair_wiring` — 5/5 pass
- `cargo test -p smelt-logical --test repair_cell --test diff_patch --test walk_coverage` — pass
  (walk_coverage required a doc-comment rewording to avoid a raw-text-scan false positive on its
  own prose)
- `cargo test -p smelt-runtime --test statement_parity --test technique_lowering` — pass, unchanged
- `cargo test -p smelt-cli --test maintenance_conformance --test explain --test explain_model` —
  pass, unchanged
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN
