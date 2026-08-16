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
| 2 | Dispatch the derived key-addressed repair cell outside the `grain: key` branch (`KeyedUpsert` → `grain: partition` fixture, red-green) | pending |
| 3 | Key-valued dirt-sets through the graph layer: key-level dirt representation alongside intervals | pending |
| 4 | Live observed-delta consumption: `--since-upstream` reads the recorded delta table; settle-bound × observed-delta "delta empty" leg | pending |
| 5 | Persisted per-source watermark with `state.mode`-aware residency; cross-model runs need no command-line landed-delta declarations | pending |
| 6 | `smelt explain`: signature headline first, per-column guarantees, derived run shape; pre-execution refusal surfacing | pending |
| 7 | Conformance extension: scheduler-driven keyed→partition cross-model recipe(s) in the generative suite | pending |
| 8 | Validate + close out: divergence bullets removed/narrowed, docs-site updated, full standing-gate sweep | pending |

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

## Blocked

_(empty)_
