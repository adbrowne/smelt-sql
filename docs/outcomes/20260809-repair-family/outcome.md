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
| 1 | Spec: repair techniques — per-group recompute + diff-then-patch semantics, admission obligations, refusal narrowing | planned |
| 2 | Delta discovery names affected keys (retraction/mutation → key set, fail-closed) | pending |
| 3 | Per-group recompute technique: derivation, admission, emitter | pending |
| 4 | `diff_patch` write pattern: registry entry, emitter, statement parity | pending |
| 5 | Wire refusal narrowing: retraction paths route to repair before full refresh | pending |
| 6 | Conformance recipes for repair + diff-patch families | pending |
| 7 | Surface: `smelt explain` rendering, docs-site update | pending |

## Decision log

<!-- Dated one-liners appended by plan/implement steps. -->

- 2026-08-09 (plan 1): outcome activated; phase table kept as scaffolded (rung-2 outcome closed clean, nothing to reshape). Phase 1 scoped spec-only: repair family as a §Semantics section, affected-key discovery owned by `model_properties.md`, `diff_patch` as a write-pattern registry entry, two new `Maintenance*` refusal codes.

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
