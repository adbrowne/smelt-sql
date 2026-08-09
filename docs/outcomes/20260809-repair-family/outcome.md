# Outcome: The repair family — per-group recompute and diff-then-patch

**Created:** 2026-08-09
**Status:** active
**Source:** `docs/research/20260809-incremental-rethink.md` §3 T-A/T-B, §6 step 2
**Spec anchors:** `docs/specs/incremental_models.md` (technique families, write-pattern registry), `docs/research/20260724-ivm-pattern-gap-catalogue.md` §A1/§C1

## The outcome

The maintenance plan gains repair techniques: when a non-invertible aggregate
receives a retraction (or a probe detects drift), smelt recomputes only the
affected groups from their bounded input slice — instead of refusing to a full
refresh. A diff-then-patch write pattern (compute the slice, diff against
stored state, write only the difference) exists as a registry entry serving
reconciliation runs and idempotent re-runs.

## Success criteria (checkable)

1. A keyed model with a non-invertible combiner over a mutable/retraction
   source derives a per-group recompute cell (affected keys → bounded
   recompute) where today it refuses (`KeyedReprocessedWindow` / full refresh).
2. Admission is proof-gated: derivable group key, bounded per-group read
   footprint, delta discovery naming the affected keys; anything unprovable
   still refuses by name.
3. `diff_patch` is a registered write pattern with a pure emitter; executed
   statements pass `cargo test -p smelt-runtime --test statement_parity`.
4. `maintenance_conformance` recipes cover retraction → per-group repair and
   reconcile-via-diff-patch, asserted against the full-refresh oracle.
5. `smelt explain` renders repair cells with their key-slice and read bound.
6. All standing gates green; walk rule holds (no new whole-text scans).

## Out of scope

- Derivation-count state for `DISTINCT`/`EXISTS` under retraction (needs the
  rung-2 state machinery pattern; schedule as its own outcome if it grows).
- Shadow-build-and-swap (T-D) and backfill choreography (T-E).

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec: repair techniques — per-group recompute + diff-then-patch semantics, admission obligations, refusal narrowing | done |
| 2 | Delta discovery names affected keys (retraction/mutation → key set, fail-closed) | done |
| 3 | Per-group recompute technique: derivation, admission, emitter | done |
| 4 | `diff_patch` write pattern: registry entry, admission, pure emitter, structural no-authoring leg | done |
| 5 | Refusal narrowing in plan derivation: retraction paths route to a repair cell, unprovable obligations refuse by name; `diff_patch` delete-leg completeness premise | done |
| 6 | Runtime lowering: per-group recompute cells execute; executed-vs-emitted `statement_parity` leg for the repair family | done |
| 7 | Runtime routing for the `diff_patch` write pin (`ChosenTechnique::DiffPatch` → emitter) + its executed-vs-emitted `statement_parity` leg | planned |
| 8 | Conformance recipes for repair + diff-patch families | pending |
| 9 | Surface: `smelt explain` rendering, docs-site update | pending |

## Decision log

<!-- Dated one-liners appended by plan/implement steps. -->

- 2026-08-09 (plan 1): outcome activated; phase table kept as scaffolded (rung-2 outcome closed clean, nothing to reshape). Phase 1 scoped spec-only: repair family as a §Semantics section, affected-key discovery owned by `model_properties.md`, `diff_patch` as a write-pattern registry entry, two new `Maintenance*` refusal codes.
- 2026-08-09 (implement 1): landed §"The repair family" in `incremental_models.md` — corner placement is column-scoped re-derivation (full read, targeted write), not a new corner; two of its three admission obligations are the *existing* obligations 4/6 cited by number, only affected-key discovery is new (obligation 7); slice completeness reuses key temporal locality rather than a new proof. `diff_patch` landed as a subsection under §"The write-pattern set is open" with its delete leg gated on the same slice-completeness premise. `model_properties.md` gained §"Affected-key discovery" (`derive_affected_keys`, fail-closed, sound-over-approximation-only). Refusal narrowing landed in §"Reprocessing" and both `KeyedReprocessedWindow`/`KeyedRetractableContribution` diagnostics prose (both spec files).

- 2026-08-09 (plan 2): no reshape — phase 1 fixed `derive_affected_keys`'s entry point and verdict shape, so the remaining rows stand as scaffolded. Phase 2 scoped proof-only (pure `smelt-logical` classifier + spec §Surface status flip); plan-cell derivation, emission and refusal wiring stay in phases 3–5. Provenance resolution reuses `analysis::fingerprint`'s walk-backed per-column leaf classifier (parameterised by an output-column filter) rather than a second lineage implementation, keeping the property-composition-walk rule intact.

- 2026-08-09 (implement 2): landed `derive_affected_keys` in
  `crates/smelt-logical/src/analysis/affected_keys.rs` — grain precedence matches
  `row_identity_with_context` exactly (declared `unique_key` else fan-out-gated proven grain,
  no `WholeRow`-style fallback). Reused `analysis::fingerprint`'s leaf classifier via
  `pub(crate)` visibility rather than a copy; introduced zero new `.contains("` sites. Flagged
  for phase 3: a grain column with zero dependency on the delta's own source is treated as
  "no requirement" — untested corner, no pinning spec sentence.

- 2026-08-09 (plan 3): one reshape — phase 5 now also owns the `smelt-runtime` lowering that
  executes a repair cell, since a derived-but-unrouted cell has nothing to lower; phase 3 gives
  the new `Technique` variant an explicit fail-loud lowering arm instead. Phase 3 scoped to
  pure `smelt-logical` machinery (variant, two refusal variants, `repair.rs` admission +
  cell derivation, emitter) plus one spec sentence resolving phase 2's flagged corner: when
  *every* grain column is independent of the delta's source the verdict is `NotDiscoverable`,
  not an unconstrained key set — the repair family never widens to a whole-table repair.

- 2026-08-09 (implement 3): landed `Technique::PerGroupRecompute`, `Refusal::
  RepairKeysNotDiscoverable`/`RepairSliceUnbounded`, `maintenance::repair::
  {admit_per_group_recompute, derive_repair_cell}` and `emit::emit_per_group_recompute` — all
  standalone, unit-proven, not yet called from `derive_maintenance_plan` (phase 5's scope).
  Fixed the "every grain column independent of source" corner directly in `affected_keys.rs`
  (the proof's sole owner), per phase 3's own spec delta. Widened `derive::LocalityInputs`/
  `SourceLink`/`project_source_link` from private to `pub` so `repair.rs` reuses the exact same
  scan-clamp derivation `derive_mutation` uses, rather than a second copy.

- 2026-08-09 (plan 4): one reshape — `statement_parity`'s *executed*-vs-emitted leg for
  `diff_patch` (and the repair cell) moves into phase 5's row, since a pattern nothing routes to
  executes no statements; phase 4 keeps the registry entry, admission, pure emitter and the
  structural no-authoring leg. Criterion 3 is therefore split across phases 4 and 5, not deferred.
  Phase 4 also decides that `diff_patch` is a write *mechanism*, not a new `Technique`: it enters
  the closed enum's namespace via a `WriteSelection::DiffPatch` arm plus a
  `ChosenTechnique::DiffPatch` variant carrying the underlying recompute technique and the
  delete-leg admission, so a pin can never silently degrade to a blanket delete+insert.

- 2026-08-09 (implement 4): landed `diff_patch` as `WriteSelection::DiffPatch` +
  `ChosenTechnique::DiffPatch { recompute: Technique, delete_leg: diff_patch::DeleteLeg }` (no new
  `Technique` variant), `maintenance::diff_patch::admit_diff_patch` (identity via
  `RowIdentity::Key`, comparability reused verbatim from `choice::resolve_write_suppression`,
  slice completeness as a caller-supplied `Result<(), String>`), and `emit::emit_diff_patch` (one
  function, conditional delete-leg statement, not two sibling emitters — the degradation is a
  per-call runtime fact, not a distinct caller population). An incomparable/unproven compared
  column refuses the whole pattern rather than degrading to an unconditional update leg (that
  would just be delete+insert with extra steps). `resolve_cell_choice`'s new `DiffPatch` arm
  always resolves `DeleteLeg::Omitted` today — the real completeness proof is phase 5's to thread
  through.

- 2026-08-09 (plan 5): one reshape — the old phase 5 split in two, since plan-derivation wiring and
  runtime lowering are independently verifiable and the combined row was too wide for one step:
  new phase 5 is derive-layer only (refusal narrowing + the `diff_patch` completeness premise phase
  4 flagged), new phase 6 owns runtime lowering and both executed-vs-emitted `statement_parity`
  legs; conformance and surface shift to 7 and 8. Nothing left the outcome. Phase 5 decides: the
  narrowing hooks `derive_new_data`'s key-grain faithful-fold *source-posture* leg (the retraction
  case criterion 1 names), repair only ever converts a refusal into a cell — never replaces an
  admitted `ColumnScopedMerge`/fold cell — a failed obligation pushes its `Refusal::Repair*`
  *alongside* the existing `NoAdmissibleTechnique`, and the `DeltaShape` is derived from the model's
  own SQL (a `MutableSnapshot` delta is a whole-row snapshot diff) rather than plumbed as a new
  world fact. The combiner-algebra leg (holistic combiner over an append-only source) is not
  narrowed — it is not a success criterion.

- 2026-08-09 (implement 5): landed the narrowing — `derive_new_data`'s faithful-fold
  source-posture failure branch now attempts `repair::admit_per_group_recompute` before refusing,
  pushing a `PerGroupRecompute` cell on success or the additive `Refusal::RepairKeysNotDiscoverable`
  / `RepairSliceUnbounded` alongside the pre-existing `NoAdmissibleTechnique` on failure.
  `derive_repair_cell` now takes the real `Trigger`; added `repair::delta_shape_for_source`
  (reuses `fingerprint_projection`'s leaf classifier, fails closed to empty columns on
  `Projection::FullRow`). `resolve_cell_choice`'s `DiffPatch` arm now grants `DeleteLeg::Complete`
  when the underlying recompute is `PerGroupRecompute` (its own key-temporal-locality premise),
  `Omitted` otherwise. Five pre-existing tests needed their refusal-count assertions widened
  (additive refusal, not a replacement) — no golden/conformance fallout otherwise.

- 2026-08-09 (plan 6): one reshape — the old phase 6 split in two, the same way phase 5 did and for
  the same reason: it carried two independent families' runtime lowering (a repair cell that
  derives itself, and a `diff_patch` write pin that only ever arrives via `resolve_cell_choice`'s
  `ChosenTechnique::DiffPatch`), each with its own `statement_parity` leg and its own execute.rs
  routing site. New phase 6 is the repair family only; new phase 7 owns `diff_patch` routing;
  conformance and surface shift to 8 and 9. Nothing left the outcome — criterion 3 stays split
  across phases 4/7. Phase 6 decides: `candidate_select` is the model's own FULL recompiled SQL
  semi-joined to the affected keys (the shape `execute_staged_membership_recompute` already uses
  for a group-complete recompute), and the cell's `ScanClamp` is pushed into the *affected-keys*
  read (a predicate on the source, where the clamp is actually defined) rather than onto the
  output wrapper, where the partition column need not appear.

- 2026-08-09 (implement 6): landed runtime lowering — `resolve_live_per_group_recompute_cell` +
  `execute_per_group_recompute` (`maintenance_driver.rs`), routed in the keyed run loop's
  window-forward branch *instead of* `execute_cumulative_aggregate` (a repair cell is an
  alternative to `KeyedFold` for the same `NewData` trigger, not a technique dispatched
  alongside it, unlike column-scoped-merge/membership-recompute). `repair_affected_keys_select`
  reuses `widened_scan_predicate` (previously test-only) with typed `TIMESTAMP` region literals
  — the one place a region endpoint is an arithmetic operand. `diagnostics.rs`'s `PerGroupRecompute`
  preview arm now builds real statements; `build_technique_statements` threads `cell: &PlanCell`
  instead of just `trigger`. `docs/specs/incremental_models.md` divergence entry narrowed to
  `diff_patch` routing only (phase 7). Flagged for phase 8: no shipped example workspace reaches
  the repair family yet — the new DuckDB tests stage their own fixture; a real conformance recipe
  needs one too.

- 2026-08-09 (plan 7): no reshape — phase 6 closed clean, phases 8/9 stand as written. Phase 7
  decides: routing extends `resolve_live_per_group_recompute_cell` with a write *mode* rather
  than adding a near-verbatim sibling resolver (a `diff_patch` write over a repair cell reads
  the identical affected-key set, candidate select and key — only the write leg differs); only
  `ChosenTechnique::DiffPatch { recompute: PerGroupRecompute }` is routable (the sole recompute
  granted `DeleteLeg::Complete`), and a `diff_patch` pin over the region `DeleteInsert` default
  fails loud by name rather than falling through to the default write. `emit_diff_patch`'s
  `partition_col`/`region` pair collapses to one caller-composed `slice_predicate` — a keyed
  aggregate output has no partition column, so a region predicate cannot express the only slice
  the routable recompute produces; no shipped statement changes, since nothing routed to this
  emitter before.

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
