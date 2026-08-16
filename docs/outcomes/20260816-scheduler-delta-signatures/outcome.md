# Outcome: The scheduler consumes delta signatures

**Created:** 2026-08-16
**Status:** active
**Source:** `docs/handoffs/2026-08-16-delta-signature-closure-programme.md` (programme outcome 2)
**Spec anchors:** `docs/specs/incremental_models.md`, `docs/specs/incremental_shapes.md`,
`docs/specs/run_state.md`

> **Operator note (design risk).** This is the highest-design-risk outcome in the programme:
> it changes the run loop's currency from whole day-intervals to typed delta components.
> Andrew reviews the first Opus-planned phase (`phases/01-plan.md`, committed by the PLAN
> step) before or shortly after the loop starts implementing it — see the handoff.

## The outcome

The DAG scheduler's currency for "what needs re-running" stops being whole day-intervals and
becomes the typed delta components the derivation layer already produces. A key-addressed repair
cell derived for a `KeyedUpsert` upstream feeding a `grain: partition` downstream is actually
dispatched — the key-addressed route works outside the `grain: key` run branch. Keyed dirt-sets
carry affected key *values*, not just key columns and provenance. `--since-upstream` reads the
recorded observed-delta table live instead of requiring the operator to restate what landed, and
a persisted per-source watermark makes cross-model incremental runs work without command-line
delta declarations. `smelt explain` becomes the verification surface for all of it: the
delta-signature headline is the first line, followed by per-column guarantees and the derived
run shape.

## Success criteria (checkable)

1. **Key-addressed dispatch outside `grain: key`.** A clockless `keyed upsert` upstream feeding
   a `grain: partition` downstream has its derived key-addressed repair cell dispatched by the
   run loop (incremental, not the correct-but-full ordinary route). Fixture-backed.
2. **Key-valued dirt-sets.** Keyed dirt carries affected key values end to end through the
   graph layer (a key-level dirt representation exists — intervals are no longer the graph's
   only currency); value-level discovery feeds the scheduler, not only the run-time mechanism.
3. **Live observed-delta consumption.** `--since-upstream` reads the recorded delta table live;
   the settle-bound × observed-delta composition gets its live "delta empty" leg.
4. **Persisted per-source watermark.** Cross-model runs no longer require the operator to state
   what landed upstream; the watermark is a state family with its `state.mode` /absent-state
   behaviour specified (building on outcome `20260816-state-residency`).
5. **`smelt explain` verification surface.** The delta-signature headline is printed first, with
   the per-column guarantee summary and the derived run shape; refusals surfaced pre-execution
   where the spec requires. Closes `incremental_models.md` "`smelt explain` does not yet print
   the delta-signature headline."
6. **Spec truth restored.** `incremental_models.md` "The scheduler does not yet consume delta
   signatures end to end" and "Delta detection for `--since-upstream` is explicit-only in v1"
   are removed or narrowed to exactly the residue that remains; `/smelt:validate
   incremental_models` shows no drift for the closed bullets.
7. All standing gates green, including `maintenance_conformance` extended to cover at least one
   scheduler-driven keyed→partition cross-model recipe.

## Out of scope

- Live fold machinery for `change_feed` delta shapes, and `change_feed` `UpstreamMutation`
  cells — decision track / Future Extensions.
- The cost model between two admissible techniques.
- Per-cell `deferral` scheduling and `diff_patch` runtime lowering (per-cell frontier
  addressing) — keyed/partition residue outcomes, after the decision track grows their scope.
- The definition-delta vertical (programme outcome 3).
- Automatic source diffing beyond the persisted watermark (snapshot diffing stays future work).

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec delta first: pin the scheduler-currency design (typed delta components, key-valued dirt, watermark semantics) in `incremental_models.md`/`run_state.md` §Design before wiring — **Andrew reviews this plan** | done |
| 2 | Dispatch the derived key-addressed repair cell outside the `grain: key` branch (`KeyedUpsert` → `grain: partition` fixture, red-green) | done |
| 3 | Key-valued dirt-sets in the graph layer: `KeyedDirt` carries resolved key values (distinct from the unresolved-symbolic form); keyed seeds enter `propagate` as pure input; composition projects keys onto each consumer's key scope, widening never narrowing; plumbed through `plan_since_upstream` | done |
| 4 | Dispatch composition in the run loop: every resolved key-addressed cell dispatches in one tick (lifts phase 2's single-edge substitution gate to a coverage gate); an uncovered inbound input still widens to the ordinary route but reports the downgrade | done |
| 5 | Propagated key restrictions reach the key-addressed cell: a request-level keyed-restriction channel, unioned (never intersected) into the affected-key relation, with `--since-upstream` passing `keyed_dirty` through | pending |
| 6 | Live consumption: `--since-upstream` reads the recorded observed-delta table live, and keyed seeds are resolved live from the group-grain sidecar diff; settle-bound × observed-delta "delta empty" leg | pending |
| 7 | Persisted per-source watermark with `state.mode`-aware residency; cross-model runs need no command-line landed-delta declarations | pending |
| 8 | `smelt explain`: signature headline first, per-column guarantees, derived run shape; pre-execution refusal surfacing | pending |
| 9 | Walk fix: `group_by_output_keys` matches `GROUP BY` keys against output aliases, not only select-item expression text (unblocks grouped-with-derived-columns recipes) | pending |
| 10 | Conformance extension: scheduler-driven keyed→partition cross-model recipe(s) in the generative suite | pending |
| 11 | Validate + close out: divergence bullets removed/narrowed, docs-site updated, full standing-gate sweep | pending |

## Decision log

- 2026-08-16: outcome activated; phase 1 planned. No reshape of the phase table — this is the
  first phase, there is no prior summary in this outcome to reshape from, and the eight rows
  map one-to-one onto success criteria 1–7 plus close-out.
- 2026-08-16 (phase 1 planning): pinned three design choices for Andrew's review rather than
  leaving them to the implementing phases — dispatch typed by *addressing* not model grain;
  key *values* seed a still-pure propagation function (rejecting backend I/O inside
  propagation, and rejecting symbolic-only dirt resolved at run time); the per-source
  watermark is a field on the existing observability-classified landed-delta family rather
  than a new correctness-classified state family (which would contradict `state.md`'s
  optionality rule).
- 2026-08-16 (phase 1 implemented): spec deltas landed in `incremental_models.md` (new
  §"Dispatch — from propagated components to run units", value-carrying "Keyed dirt-sets and
  the narrowed refusal", three §Design paragraphs, CLI/run-flags wording, two reworded Known
  Divergences bullets) and `run_state.md`/`state.md` (new §"Per-source watermark", inventory
  row updated). One incidental edit to `incremental_shapes.md` to distinguish the new
  propagation watermark from the rejected "watermark store" design paragraphs it would
  otherwise appear to contradict. No production code touched, per plan. Andrew's review of
  `phases/01-plan.md` is still pending/parallel to this implementation, per the outcome's
  operator note — phases 2+ implement exactly what this phase pinned, so a requested change
  here should land as a follow-up spec amendment before phase 2 proceeds far.
- 2026-08-16 (phase 2 planning): reshape — phase 3's row now explicitly carries dispatch
  *composition* (a model receiving several components in one tick). Phase 2 substitutes the
  key-addressed cell for the ordinary route only when every inbound ref of the model is a
  key-addressed edge that resolved a cell; a partition-grain downstream with an additional
  uncovered source keeps its ordinary route rather than risking a silently dropped component.
  That residue serves success criteria 1–2, so it stays inside the outcome (phase 3) rather
  than being deferred out. Confirmed the derivation half already admits the cell for a
  `grain: partition` downstream (`derive.rs::append_model_edge_cells`, clock route inapplicable
  for a clockless upstream) — phase 2 is a dispatch-only gap, and the
  `keyed_partition_sink_dag` testkit fixture plus its oracle test already exist to extend.
- 2026-08-16 (phase 2 implemented): dispatch lands — the key-addressed cell now runs for a
  clockless `keyed upsert` → `grain: partition` edge when the substitution gate holds (single
  resolved edge, no other inbound ref). Found and worked around (not fixed) a pre-existing
  `smelt-logical` walk gap along the way: `group_by_output_keys` matches `GROUP BY` keys against
  select-item expression text, not output alias, so grouping by a projected alias fails grain
  proof entirely rather than dropping just that column — flagged for a future phase, see
  `phases/02-summary.md` "For the next planner". No reshape of the phase table.

- 2026-08-16 (phase 3 planning): reshape — old row 3 split into two (3: key-valued dirt
  representation + pure composition + plumbing, in `smelt-logical`/`smelt-runtime::propagation`;
  4: dispatch composition in the run loop, lifting phase 2's substitution gate). One phase
  cannot honestly carry both the graph-layer currency change and the run-loop dispatch change.
  Live *keyed-seed* resolution (the backend sidecar read that fills the seeds) folded into the
  live-consumption row (now 5) beside the observed-delta read — both are the same "read the
  warehouse instead of trusting the command line" work, and criterion 2's "value-level discovery
  feeds the scheduler" is only met once that lands. Added row 8 for the
  `group_by_output_keys` alias gap phase 2's summary flagged: it serves criteria 1/7 (grouped
  recipes with derived columns currently hard-refuse grain proof), so it stays inside the
  outcome rather than being deferred out; placed before the conformance extension so recipes may
  use alias grouping. Old rows 4–8 renumbered 5–10; nothing dropped.

- 2026-08-16 (phase 3 implemented): `KeyValues`/`KeyedDirt::values`, `Edge::consumer_key_scope`,
  `propagate_with_keys` (with `propagate` as a delegating wrapper), and the keyed-only-node visit
  fix land in `smelt-logical::maintenance::propagate`; `smelt-runtime::propagation` folds
  `PlanCell::key_scope` into `Edge::consumer_key_scope` and adds
  `plan_since_upstream_with_keyed_seeds` plus `SinceUpstreamPlan::keyed_dirty`. All six planned
  tests pass; no reshape of the phase table. Fan-in merge across more than one admitted inbound
  keyed edge is implemented but untested (no fixture in this phase's list exercised it) —
  flagged for phase 4+ if a real fan-in scenario surfaces.

- 2026-08-16 (operator review of phase 1, follow-up spec amendment): Andrew's review of
  `phases/01-plan.md` upheld the three pinned design decisions and landed four amendments
  before phase 4+ builds on them. (1) Absent-watermark behaviour contradiction resolved in
  favour of the CLI's **refuse-loudly leg**: a source named with neither `--landed` nor a
  watermark propagates nothing, naming the missing watermark — the run_state bullet's
  "recompute the full dirty set" degradation is gone (unbounded, unasked-for cost), and
  `state.md` §"The optionality rule" swaps its now-wrong forward-propagation example for
  `--auto`. (2) Watermark granularity pinned: per source, advanced only by a run that
  completed every consumer; selective runs stall it, never advance it; per-`(source,
  consumer)` recorded as rejected-for-now in §Design. (3) `watermark → now` semantics for
  external sources pinned: the span itself is the landed delta; observed-delta refinement
  applies only where a record exists (model upstreams — only smelt's conditional-write path
  writes one). (4) The stale "no key-level dirt representation exists" clause removed from
  the Graph-layer-gaps divergence bullet (phase 3 landed the representation; the scheduler
  bullet owns the remaining residue). Phases 5–6 implement the amended semantics.

- 2026-08-16 (phase 4 planning): reshape — old row 4 split into two (4: dispatch composition,
  i.e. resolving and dispatching every key-addressed cell a model's inbound edges yield, plus the
  visible widen-never-narrow downgrade when an input is uncovered; 5: the propagated key
  restriction actually reaching the cell). The two halves are independent seams — composition is
  entirely inside `execute.rs`/`maintenance_driver.rs`, while the restriction needs a new
  request-level channel (`ExecuteRequest`, ~70 struct literals) plus an emitter parameter —
  and phase 3's precedent ("one phase cannot honestly carry both") applies. Nothing left the
  outcome; old rows 5–10 renumbered 6–11. Also pinned for row 5, so the implementer does not have
  to rediscover it: the restriction must be **unioned** with the sidecar-discovered key set, never
  intersected — the sidecar refresh runs in the same transaction as the write, so narrowing the
  repaired set would advance the comparandum past keys that were never consumed (the wrong-and-quiet
  outcome §"Upstream model edges" forbids). Phase 4 also lands the reporter-visible downgrade the
  spec's §"Widen-never-narrow at dispatch" already requires and phase 2 left silent; the
  `smelt explain` half of that visibility stays with row 8.

- 2026-08-16 (phase 4 implemented): `resolve_live_key_addressed_model_edge_cells` (plural)
  lands in `maintenance_driver.rs`; both `execute.rs` dispatch sites compose every resolved
  cell in one tick; the non-keyed site is now a coverage gate (licensed only when EVERY inbound
  ref resolved a key-addressed cell) with a `RunReporter::dispatch_widened` downgrade report on
  refusal. Caught and fixed a real bug along the way: `EventSink` (the wavefront scheduler's
  per-model event buffer) silently drops any `RunReporter` method it does not explicitly buffer
  and replay — `dispatch_widened` needed its own `ReporterEvent` variant + buffer/replay arms,
  or it would have silently no-op'd under the real concurrent run loop. Multi-edge admission
  (both the new unit test and the new e2e fixture) required a COALESCE/composite-grain SQL
  shape rather than a plain equi-join — `derive_affected_keys`'s provenance walk traces literal
  SELECT-list lineage, not join-predicate equality. No reshape of the phase table.

## Blocked

_(empty)_
