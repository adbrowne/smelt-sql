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
- Disambiguating same-named consumer-read columns across two upstream groups
  (`referenced_column_names` is name-only): a coarseness that only ever widens, and no success
  criterion depends on the finer resolution.
- Scaling `derive_workspace_output_deltas` below its `O(models^2)` worst case: performance work,
  not correctness; flag only if a real workspace shows it as a hot path.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec: output-delta types, transfer rules, typed edges, the narrowed keyed refusal | done |
| 2 | Walk transfer rules for the output-delta verdict per column group | done |
| 3 | Edge typing in the propagation layer; adjoint property preserved for window addressing | done |
| 4 | Consumer-side fold over an upstream keyed-upsert delta (model-edge change-feed) | done |
| 5 | Keyed dirt-set propagation for admitted shapes | done |
| 6 | Key-addressed model-edge cells: clockless keyed upstream, keyed-downstream fold (plan derivation) | done |
| 7 | Lowering + execution of a key-addressed model-edge cell (statement emission, driver) | done |
| 8 | Conformance recipes: end-to-end keyed chain vs full-refresh oracle | done |
| 9 | `smelt-db` derives typed model edges (`ModelEdge.output_shape`) so explain/diagnostics see keyed edges | done |
| 10 | Surface: explain edge delta-type rendering, degradation reasons, docs-site update | planned |
| 11 | Model-reference leaf resolves a bare `smelt.<addr>` upstream ref through `model_verdicts` (3+-hop chains) | pending |

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

- 2026-08-10 — Phase 5 planned; no phase-table reshape. Phase 4's two flagged items are neither
  success-criteria work nor bugs, so they are recorded under Out of scope rather than given rows:
  `referenced_column_names`'s name-only read-column filter (same-named columns from two upstream
  groups undisambiguated) and `derive_workspace_output_deltas`'s unconditional `O(models^2)`
  worst-case fold. Phase 5's shape decision taken in-plan: the keyed dirt-set is an **additive
  symbolic channel** on `Propagation` (key columns + provenance) alongside the existing interval
  maps, not a rewrite of dirt into a sum type — the value-level affected-key set stays with the
  run-time discovery mechanism, and a keyed-addressed edge into a *clocked* consumer still emits
  `DayInterval::WHOLE` so widen-never-narrow holds without inventing an axis.

- 2026-08-10 — Phase 5 implemented: `classify_keyed_edges` (replacing `refuse_keyed_nodes`) admits
  an edge touching a `PartitionGrain::Keyed` endpoint when a component carries `Addressing::Keyed`,
  routing it through a new additive keyed channel (`Propagation::{per_edge_keys, keyed_dirty}`)
  instead of interval math; a clocked/unclocked consumer of a keyed origin still gets
  `DayInterval::WHOLE` so it runs. `smelt-runtime::refuse_bare_keyed_origins` narrowed the same
  way, consulting the model's own derived output-delta shape. No phase-table reshape — phase 5's
  scope matched the plan; phase 6 (end-to-end conformance chains) is now unblocked.

- 2026-08-10 — Phase 6 planned **with a reshape**: the old single row 6 ("conformance recipes")
  assumed a keyed upstream could already reach its consumer through the real plan derivation. It
  cannot, and phase 5's own fixture is the evidence — it had to hand a `grain: key` model a
  synthetic `timeseries:` block "purely so the downstream's `ModelEdge` derivation gets a
  `clock_col`". Reading the derivation confirms two hard stops in
  `maintenance::derive::append_model_edge_cells`: an edge with `clock_col: None` is refused
  `ReachNotDerivable`, and a downstream with no `output_partition_col` (i.e. any keyed consumer)
  returns before any cell is built ("its model-edge creation would be a keyed fold, deferred").
  Both are exactly the change-feed case success criterion 3 names, so the work is not deferrable
  out — it gets rows. Old row 6 splits into: 6 key-addressed model-edge cells (plan derivation),
  7 lowering/execution of such a cell, 8 the conformance recipes, 9 the surface/docs row
  (formerly 7). Design call taken in-plan, not blocked: the key-addressed restriction is an
  **additive** `PlanCell::key_scope` alongside the interval-shaped `scans` (the phase-5 precedent
  of an additive keyed channel rather than a sum-typed rewrite), and the technique reused is the
  existing `Technique::PerGroupRecompute` — the repair family already means "recompute and write
  only the affected key groups", which is what folding an upstream upsert delta is.

- 2026-08-10 — Phase 6 implemented: `append_model_edge_cells` admits a key-addressed
  `PerGroupRecompute` cell (`admit_key_addressed_recompute`, reusing `derive_affected_keys`) for
  a `KeyedUpsert`-shaped edge the existing clock-based route can't serve — a clockless upstream,
  or a keyed-grain downstream — narrowing `ReachNotDerivable` rather than replacing it. The route
  is taken only when the clock-based route has nothing to admit anyway (not unconditionally for
  every `KeyedUpsert` edge): an unconditional rule broke every pre-existing clocked fixture, since
  those downstreams never declare a `unique_key`/provable grain the new route needs but the old
  one didn't. In-phase bug fix (blocking this phase's own real-graph test):
  `analysis::fingerprint::relation_matches_source` only stripped a `sources.` breadcrumb, never
  `models.`, so affected-key discovery over any `smelt.models.<addr>`-style model ref (a model
  living directly under `models/`, no subdirectory) always resolved to "touches no columns" —
  fixed to strip either breadcrumb (provably additive, full suite green after). No phase-table
  reshape — phase 6's scope matched the plan; `smelt-db`'s `smelt explain` surface and phase 7's
  lowering are unaffected and remain their own rows' scope (see `phases/06-summary.md` "For the
  next planner").

- 2026-08-10 — Phase 7 planned; no phase-table reshape. Phase 6's three "for the next planner"
  items are all placed without deferring anything: the lowering *is* this phase, `execute.rs`'s
  `model_edges_for` `output_shape: None` wiring is a phase-7 task (without it the driver can never
  see a cell it derives), and the `smelt explain` `ref_model_edge` rendering stays phase 9's own
  surface row. Two design calls taken in-plan rather than blocked: (a) a key-addressed model
  edge discovers its affected key set from the **group-grain fingerprint sidecar over the
  upstream's output table** at the upstream's key grain — phase 5's keyed dirt channel is
  symbolic, not value-level, and `repair_affected_keys_select`'s clamp-less form would scan every
  key and degenerate to a full refresh, which success criterion 3 forbids; the sidecar is the
  existing mechanism for exactly this posture (a clockless keyed upstream is a mutable snapshot
  from the consumer's view) and works when the upstream did not run this invocation. DuckDB-only,
  failing loud, matching the sidecar's existing posture. (b) The upstream's changed keys are
  projected through the upstream relation onto the downstream key columns `KeyScope::keys` names;
  a `key_scope` key the upstream relation does not carry is a fail-loud refusal, never a widening.
  The latent `skeleton_closure.rs` breadcrumb gap phase 6 flagged stays a **conditional,
  red-test-first task inside phase 7** (fixed only if a fixture trips it), not a speculative fix
  and not a deferral.

- 2026-08-10 — Phase 7 implemented: `emit_key_addressed_affected_keys_select` (`smelt-logical`),
  `resolve_live_key_addressed_model_edge_cell` + `resolve_key_addressed_affected_keys` +
  `execute_key_addressed_model_edge_cell` (`smelt-runtime::maintenance_driver`), and
  `execute.rs`'s keyed-run-loop dispatch (checked BEFORE the `(start_date, end_date)` match rather
  than nested inside its window-forward arm — a clockless upstream typically drives its downstream
  into the snapshot-reconcile shape, which has no run window to match on at all, so nesting inside
  the window-forward arm would make the new route unreachable for the archetypal case). The
  `skeleton_closure.rs` conditional task was not needed — no fixture tripped it. Real-DuckDB
  two-model chain proven end-to-end (`key_addressed_model_edge_lowering.rs`) plus a
  `statement_parity` leg. No phase-table reshape — phase 7's scope matched the plan, with one
  scope note for the next planner: live dispatch reaches only a `grain: key` downstream today; a
  `KeyedUpsert` upstream feeding a `grain: partition` downstream (also admitted by phase 6's plan
  derivation) has no live dispatch yet (see `phases/07-summary.md` "For the next planner").

- 2026-08-10 — Phase 8 planned; no phase-table reshape. Phase 7's one flagged scope note — a
  `KeyedUpsert` upstream feeding a **partition-grain** downstream derives a key-addressed cell
  that the run loop never dispatches — is handled inside phase 8 rather than given its own row
  or silently dropped: no success criterion depends on that combination being *incremental*
  (criterion 3 names a two-model chain, proven for the keyed-grain downstream in phase 7), but
  the inert-cell divergence is real, so phase 8 pins its **correctness** with a conformance
  recipe against the oracle, records it in `docs/specs/incremental_models.md` §Known
  Divergences, and registers it as a `KnownBug` staleness entry in the conformance divergence
  registry — the mechanism that exists for exactly this ("a gap this suite discovered and
  deliberately did not fix yet", auto-stale the moment dispatch widens). The
  `skeleton_closure.rs` `sources.`-only breadcrumb gap stays untouched and unregistered: still
  latent, still no fixture reaching it, and speculative fixes were already ruled out in phase 7.

- 2026-08-10 — Phase 8 implemented: `keyed_chain_dag()`/`keyed_partition_sink_dag()` (new
  generated fixtures, `smelt-maintenance-testkit::dag`) and 4 new `crates/smelt-cli/tests/
  maintenance_conformance/dags.rs` tests lift phase 7's hand-typed proof into the standing
  generative gate — a real two-model clockless keyed chain folds end-to-end against a
  full-refresh oracle, touching only the changed keys, and the flagged inert-cell combination
  (`KeyedUpsert` upstream → `grain: partition` downstream) is pinned correct-but-non-incremental.
  A `KnownBug` registry entry + spec §Known Divergences entry track the gap. In-phase discovery
  (not this phase's own scope, flagged for phase 9): `smelt-db::lib.rs`'s own model-edge
  construction hard-codes `output_shape: None` unconditionally, so `smelt explain`/diagnostics
  never see a `KeyedUpsert` edge at all — three independent model-edge constructions exist today
  (`smelt-runtime::propagation.rs`, `smelt-runtime::execute.rs::model_edges_for`, both correct;
  `smelt-db::lib.rs`, a stub), a materially bigger gap than phase 9's plan (written before this
  discovery) may have assumed. No phase-table reshape — phase 8's scope matched the plan, with
  the divergence wording narrowed to what's directly verifiable (run-loop dispatch gating, not
  an unverifiable "plan admits a cell" claim — see `phases/08-summary.md` "For the next
  planner").

- 2026-08-10 — Phase 9 planned **with a reshape**: the old single row 9 ("surface: explain edge
  rendering, docs-site update") assumed `smelt explain` already sees a typed edge and only needs
  to render it. Phase 8's discovery says otherwise and is verified —
  `crates/smelt-db/src/lib.rs`'s `ref_model_edge` hard-codes `output_shape: None`, so the plan
  report behind `smelt explain`/diagnostics never derives a `KeyedUpsert` edge at all and
  `append_model_edge_cells`' key-addressed loop silently skips there. Rendering a field that is
  structurally always `None` would satisfy criterion 5 in letter only, so the row splits: 9
  wires the derivation into the Salsa layer (making `smelt explain`'s plan report agree with the
  run loop's), 10 renders the delta type and its degradation reason and updates docs-site.
  Design call taken in-plan rather than blocked: the Salsa side assembles `ModelDeltaInput`s for
  the transitively-referenced models and makes ONE call to the existing pure
  `derive_workspace_output_deltas`, rather than recursing a Salsa query per model reference — the
  pure fold is already bounded-pass cycle-safe, whereas Salsa recursion over a cyclic model-ref
  graph would panic.

- 2026-08-10 — Phase 9 implemented: `ref_model_edge` (`crates/smelt-db/src/lib.rs`) derives
  `output_shape` for real instead of hard-coding `None` — a new `model_delta_inputs` helper
  walks model refs transitively from the Salsa `file` (address-deduplicated, terminating over a
  cycle by construction) to build the cross-model verdict map `derive_workspace_output_deltas`
  folds once per report, and a new public `model_edges_for(db, workspace, file)` entry point
  replaces the inline assembly so `smelt explain`'s plan report and any other caller (and this
  phase's own tests, which need to observe `output_shape` directly since `MaintenancePlan`
  doesn't retain the input edges) read the same edges. `docs/specs/incremental_models.md`
  §Known Divergences narrowed accordingly. No phase-table reshape — phase 9's scope matched the
  plan, with one latent gap flagged for the next planner (not this phase's own regression): a
  model-reference leaf inside an upstream's own SQL only resolves through `model_verdicts` when
  the SQL literally spells `smelt.models.<addr>`; every current fixture's bare `smelt.<addr>`
  form bypasses it for a 3+-hop chain (pre-existing since phase 4, unexercised by any current
  conformance recipe). Phase 10 (surface: explain edge rendering, docs-site update) is unblocked.

- 2026-08-10 — Phase 10 planned **with a reshape**: phase 9's flagged latent gap — a
  model-reference leaf only resolves through `model_verdicts` when the upstream's SQL literally
  spells `smelt.models.<addr>`, so a bare `smelt.<addr>` ref inside an upstream's own SQL (the
  form every fixture and the dag generator use) degrades to `General` for a 3+-hop chain — gets
  its own row 11 rather than an Out-of-scope line. It is not merely a coarseness: the phase-1/2
  transfer table registers "a model-reference leaf takes the referenced model's own verdict" as a
  rule, so today's behaviour diverges from a registered transfer rule (criterion 1) and stops the
  outcome's headline composition at 2 hops. Fail-closed, so no correctness bug — which is why it
  sits after the surface row rather than before it. Phase 10 itself stays surface-only: rendering
  the delta type per edge (model edges from phase 9's `ModelEdge.output_shape`, source edges from
  the leaf seed so no inbound edge renders untyped), the degradation reason, the key-addressed
  cell's sidecar discovery line, spec Surface, and docs-site.

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
