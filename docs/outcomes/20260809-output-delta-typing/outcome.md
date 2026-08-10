# Outcome: Output-delta typing — compositional incrementality across the DAG

**Created:** 2026-08-09
**Status:** active
**Source:** `docs/research/20260809-incremental-rethink.md` §2 P-B/P-E, §3 T-C, §4.1, §6 step 4
**Spec anchors:** `docs/specs/incremental_models.md` (graph layer, input-delta discovery), `docs/specs/model_properties.md` (delta-shape lattice)

## The outcome

Each model derives, per column group, the **shape of change it emits** —
`append-only within window` ⊑ `keyed upsert` ⊑ `general` — via walk transfer
rules. DAG edges carry typed deltas instead of only day-interval dirt, so a
model consuming a maintained keyed upstream folds that upstream's emitted
delta directly (the change-feed case), and incrementality composes end-to-end
through a chain instead of stopping at each model. Day intervals survive as
the addressing of one delta type, not the universal currency.

## Success criteria (checkable)

1. The walk derives an output-delta verdict per column group with registered
   transfer rules (selection/projection/UNION ALL preserve append-only; keyed
   aggregation over append-only emits keyed upsert; unregistered operators
   fail closed to `general`).
2. Propagation edges are typed: (delta shape × addressing × column set);
   the existing day-interval forward/backward maps are the window-addressed
   case and their adjoint property still holds.
3. A two-model chain — keyed maintained upstream → consuming model — is
   maintained incrementally end-to-end in the conformance gate (the consumer
   folds the upstream's upsert delta; no full-input rescan), matching the
   full-refresh oracle.
4. Keyed dirt-sets replace the blanket keyed-node propagation refusal for
   admitted shapes; the refusal survives, narrowed, for `general`.
5. `smelt explain` renders each edge's delta type; refusals name the operator
   that degraded the type.
6. All standing gates green; walk_coverage includes the new transfer rules.

## Out of scope

- Streaming/micro-batch lowering of typed deltas (later kind over the kernel).
- Engine-native change feeds (CDC ingestion) — smelt-derived deltas only.
- Column-scoped dirt beyond what edge typing gives directly.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec: output-delta types, transfer rules, typed edges, the narrowed keyed refusal | planned |
| 2 | Walk transfer rules for the output-delta verdict per column group | pending |
| 3 | Edge typing in the propagation layer; adjoint property preserved for window addressing | pending |
| 4 | Consumer-side fold over an upstream keyed-upsert delta (model-edge change-feed) | pending |
| 5 | Keyed dirt-set propagation for admitted shapes | pending |
| 6 | Conformance recipes: end-to-end incremental chains vs full-refresh oracle | pending |
| 7 | Surface: explain edge rendering, docs-site update | pending |

## Decision log

- 2026-08-09 — **Delta type is per column group, not per model** (rethink §6 open question 1, settled with Andrew): edges are vector-typed — one typed component (shape × addressing × columns) per column group the consumer reads, projected through the consumer's sensitivity. Per-model scalar typing was rejected because the meet over groups lets one mutable group degrade a model's append-only groups to `general`, blocking composition for mixed-shape models.

- 2026-08-10 — Outcome activated; phase table unchanged (no prior phase summary to reshape against). Phase 1 scoped as spec-only: the `smelt explain` edge rendering stays in phase 7 with its docs-site update, so the surface spec delta lands next to the code that produces it.

<!-- Dated one-liners appended by plan/implement steps. -->

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
