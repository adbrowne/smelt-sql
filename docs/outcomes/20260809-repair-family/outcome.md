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
| 4 | `diff_patch` write pattern: registry entry, emitter, statement parity | pending |
| 5 | Wire refusal narrowing + runtime lowering: retraction paths route to repair, cells execute | pending |
| 6 | Conformance recipes for repair + diff-patch families | pending |
| 7 | Surface: `smelt explain` rendering, docs-site update | pending |

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

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
