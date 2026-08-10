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
| 1 | Spec: output-delta types, transfer rules, typed edges, the narrowed keyed refusal | done |
| 2 | Walk transfer rules for the output-delta verdict per column group | done |
| 3 | Edge typing in the propagation layer; adjoint property preserved for window addressing | done |
| 4 | Consumer-side fold over an upstream keyed-upsert delta (model-edge change-feed) | done |
| 5 | Keyed dirt-set propagation for admitted shapes | pending |
| 6 | Conformance recipes: end-to-end incremental chains vs full-refresh oracle | pending |
| 7 | Surface: explain edge rendering, docs-site update | pending |

## Decision log

- 2026-08-09 — **Delta type is per column group, not per model** (rethink §6 open question 1, settled with Andrew): edges are vector-typed — one typed component (shape × addressing × columns) per column group the consumer reads, projected through the consumer's sensitivity. Per-model scalar typing was rejected because the meet over groups lets one mutable group degrade a model's append-only groups to `general`, blocking composition for mixed-shape models.

- 2026-08-10 — Outcome activated; phase table unchanged (no prior phase summary to reshape against). Phase 1 scoped as spec-only: the `smelt explain` edge rendering stays in phase 7 with its docs-site update, so the surface spec delta lands next to the code that produces it.

- 2026-08-10 — Phase 1 implemented: transfer-rule table rows that preserve the input shape spell
  out all three lattice names explicitly rather than saying "preserves the input shape", so the
  table stays machine-checkable per row (`crates/smelt-logical/tests/output_delta_spec.rs`).

- 2026-08-10 — Phase 2 planned; no phase-table reshape (phase 1 surfaced nothing out of scope). The
  phase-1 transfer table has no **leaf** row, so phase 2 carries a small spec delta adding one:
  a base relation seeds its shape from the source's declared mutation profile (append_only+clock ⇒
  `AppendOnlyWindow`, change_feed+`delta_identity` ⇒ `KeyedUpsert`, everything else ⇒ `General`),
  mirroring `input_delta_discovery`'s fail-closed default. A model-reference leaf takes the
  referenced model's own verdict — the hook phase 4's consumer fold reads.

- 2026-08-10 — Phase 2 implemented: `crates/smelt-logical/src/analysis/output_delta.rs` builds
  `OutputDelta` (the three-level lattice + degrade-only `meet`) and `OutputDeltaTransfer` (a
  `Transfer` impl over the shared walk) covering every transfer-rule-table row — leaf seeding
  from declared mutation profile, selection/projection pass-through, `UNION ALL` meet, `GROUP
  BY`/`DISTINCT` keyed-upsert promotion, join meet + `OneToMany` degrade (reusing
  `join_shape::fan_out`), window-column isolation, fail-closed default naming the construct — plus
  `derive_output_delta`, which folds per-column verdicts to one per `ColumnGroup` by reusing the
  existing `maintenance::grouping::derive_column_groups`. Resolution is per column *reference*
  (each embedded column ref chased and meet-folded independently), not per whole scope, which is
  what lets two differently-shaped column groups coexist inside one joined scope. No
  phase-table reshape — phase 3 (edge typing) is unblocked with a working entry point; the
  `SourceFacts`-from-declared-sources adapter and the model-reference cross-model wiring are both
  still open, flagged for phase 3/4 in `phases/02-summary.md`.

- 2026-08-10 — Phase 3 planned; no phase-table reshape. Phase 2's two flagged gaps are placed:
  the `SourceInfo` → `output_delta::SourceFacts` adapter lands in phase 3 (edge typing is its
  first real caller), and the cross-model verdict map stays in phase 4 with the consumer fold.
  Phase 3 keeps typed components **advisory** — interval math is unchanged, so the adjoint
  property is re-asserted rather than re-derived; acting on non-window components is phase 5.

- 2026-08-10 — Phase 3 implemented: `maintenance::edge_type::type_edge` derives one typed
  `EdgeComponent` per upstream column group the consumer reads, projecting `AppendOnlyWindow`
  through the consumer's own derived column groups (degrading to `WholeModel` when the axis isn't
  carried forward) and `KeyedUpsert`/`General` unconditionally to `Keyed`/`WholeModel`.
  `propagate::Edge` carries the vector as an advisory field; interval math and the adjoint
  property are unchanged and re-pinned by 3 new tests. `SourceFacts::from_source_info` lands but
  is not yet called from production code — that, and wiring `type_edge` into
  `build_forward_graph`, are phase 4's job alongside the consumer-side fold.

- 2026-08-10 — Phase 4 planned; no phase-table reshape (phase 3's scope matched its plan and its
  two "for the next planner" items — the cross-model verdict fold and a real `type_edge` caller —
  are exactly phase 4's body). One design call taken in-plan rather than blocked:
  `OutputDeltaTransfer::model_verdicts` becomes **per output column** (`OutputDeltaFacts`), not a
  scalar per model, so a model-reference leaf resolves per column reference. A scalar would have
  meet-folded a mixed-shape upstream to its worst group, which is exactly what the 2026-08-09
  per-column-group decision rejects. Typed components stay advisory for dirt (phase 5 acts).

- 2026-08-10 — Phase 4 implemented: `derive_workspace_output_deltas` folds `OutputDeltaFacts`
  across real model references (bounded fixed-point pass), and `build_forward_graph` now calls
  `type_edge` for every real edge — `Edge.components` is non-empty in production. Fixed a
  pre-existing bug in-phase (blocking this phase's own success criterion 3): a `smelt.models.*`
  ref's segments carry the literal `models` keyword, which `derive_clamp_and_locality_pass`'s addr
  computation never stripped, so no model-edge maintenance cell had ever been derived through the
  real graph builder for any workspace. `derive_consumer_column_groups` also gained a synthetic
  skeleton-column group so `type_edge`'s window-axis carriage check can find a declared
  `timeseries.partition_column`. No phase-table reshape — phase 4's scope matched the plan.

<!-- Dated one-liners appended by plan/implement steps. -->

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
