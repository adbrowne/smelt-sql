# Phase 1 plan — pin the scheduler-currency design (spec-only)

> **Andrew reviews this plan** before/shortly after the loop implements it. It is the
> design-risk gate for the whole outcome: phases 2–7 implement exactly what this phase pins.

## Objective

Pin, in the specs, the three design decisions the rest of this outcome implements: (a) how a
propagated **typed delta component** becomes a dispatched run unit — including a key-addressed
component dispatching its repair cell outside the `grain: key` run branch; (b) what a
**key-valued dirt-set** is and where the values enter a *pure* propagation function; (c) the
**persisted per-source watermark** and live observed-delta consumption, with `state.mode` and
absent-state behaviour. No production code changes. Advances success criteria 1–5 by making
them buildable, and pre-stages criterion 6 (the divergence bullets are narrowed by the phases
that close them, not here).

## Spec delta (spec-first; this phase *is* the spec edit)

1. **`docs/specs/incremental_models.md` §Semantics → new subsection "Dispatch — from
   propagated components to run units"**, placed after §"The graph layer". Normative content:
   - The run loop's currency is the **typed component vector** on each edge, not day-intervals.
     A propagation result yields, per `(model, upstream)` edge, a set of **run units**: one per
     component, each carrying the component's addressing (window / keyed / whole-model) and its
     restriction (interval set, key set, or "everything").
   - **Dispatch is keyed by the component's addressing, never by the downstream model's
     grain.** A `Keyed` component dispatches the derived key-addressed repair cell
     (`Technique::PerGroupRecompute`, §"The graph layer" → key-addressed model edges) whatever
     the downstream's `grain` is — a `grain: partition` downstream of a clockless
     `keyed upsert` upstream is the named example. Routing a key-addressed component through
     the ordinary whole-model run route is correct-but-not-incremental and is a defect against
     this paragraph, not an acceptable fallback.
   - **Widen-never-narrow at dispatch**: a component whose cell cannot be derived (no
     admissible technique for that addressing on that target) degrades to the coarsest run unit
     the consumer can act on and *says so* (an explain-visible downgrade), never to nothing and
     never silently.
   - A model receiving several components in one tick dispatches each; per-edge dirt keying
     (§Design "Per-edge dirt keys trigger cells") is unchanged — components refine it, they do
     not replace it.
2. **`docs/specs/incremental_models.md` §Semantics → "The graph layer" → "Keyed dirt-sets and
   the narrowed refusal"**: replace the symbolic-only reading with the value-carrying one:
   - A keyed dirt-set carries the **affected key values** (plus the key columns and the
     provenance node it came from). Propagation stays a **pure function**: key values enter it
     as *seed* input exactly as landed intervals do — the caller resolves them once
     (§"Affected-key discovery" / the group-grain fingerprint-sidecar diff over the upstream's
     output table) and passes them in; propagation composes them through edges by projecting
     the upstream's key columns onto each consumer's own key scope.
   - **Composition rules**: a keyed component into a keyed consumer whose key scope the
     projection resolves stays key-valued; one whose keys cannot be resolved through the
     consumer's grain widens to whole-model dirt for that consumer (never nothing, never a
     silent key drop) — the existing `MaintenanceRepairKeysNotDiscoverable` refusal continues
     to govern the *cell*, this rule governs the *dirt*.
   - **Unresolved seeds**: a keyed edge whose values were not resolvable (non-DuckDB target, no
     sidecar) propagates the symbolic form and widens at dispatch — the honest degradation, not
     an empty key set. Empty-and-resolved (nothing changed) and unresolved are distinct, the
     same way an empty observed delta and an absent one are.
3. **`docs/specs/run_state.md` §Surface → "Relationship to the reconciliation ledger" /
   §"`.smelt/` directory layout"**, plus `docs/specs/state.md` §"The state-structure
   inventory": pin the **per-source watermark** as a *field on the existing landed-delta
   record family*, not a new family — per source address, the bound propagation has already
   consumed through. Content to pin:
   - Written in the same locked, atomic write as the run's landed-delta record, advanced only
     on a run that completed the models consuming that source; a failed run never advances it.
   - Classification: **observability**, same as the landed-delta record it lives in — so
     `state.mode: stateless` has none, and the absent-state behaviour is the existing
     degradation contract (`state.md` §"The degradation contract"): recompute the full dirty
     set and report why, never silently propagate nothing.
   - `smelt run --since-upstream` with no `--landed` for a source reads
     `watermark → now` from the recorded **observed-delta** table live (`incremental_models.md`
     §"Observed deltas on model edges"); an explicit `--landed` always overrides. A recorded
     **empty** delta over that span is a real fact — the "delta empty" no-op leg of the
     settle-bound × observed-delta composition — distinct from an absent record, which falls
     back to the run's written window.
   - §Surface "CLI"/"Run flags" in `incremental_models.md`: `--landed` becomes optional per
     source when a watermark exists; the sentence "smelt does not currently discover what is
     new on its own … automatic watermark discovery is §Future Extensions" narrows to
     *snapshot diffing of external sources* (that stays future work), since the watermark
     itself is now surface.
4. **§Design paragraphs** (one each, `incremental_models.md` §Design), recording what was
   rejected: *"Dispatch is typed by addressing, not by model grain"* (rejected: the per-grain
   run branch, which is why the key-addressed route was unreachable from `grain: partition`);
   *"Key values seed propagation; propagation stays pure"* (rejected: resolving keys inside
   propagation via backend I/O — breaks the pure-derivation invariant and Salsa purity;
   rejected also: keeping dirt symbolic and resolving only at run time — the scheduler then
   cannot size or skip work); *"The watermark is a field on the landed-delta family, not a new
   one"* (rejected: a new correctness-classified state family — it would make forward
   propagation require state, contradicting the optionality rule).
5. **No Known Divergences bullet is deleted in this phase.** Where a bullet is now merely
   *stale in framing* (the "keyed dirt carries key columns, not values" clause), it is
   reworded to point at the pinned design as the target, not removed — removal belongs to the
   phase that lands the behaviour.

## Tests

Spec-only phase; the gates are doc-consistency, not TDD over behaviour.

- `cargo test -p smelt-cli --test state_docs` — the state-doctrine doc gate still passes with
  the amended inventory table (watermark listed on the landed-delta row).
- `cargo test -p smelt-cli --test example_diagnostics` — unchanged, guards no accidental
  fixture/code edit.
- Timeless-oracle grep: `rg -n "Phase [A-Z0-9]" docs/specs/incremental_models.md
  docs/specs/run_state.md docs/specs/state.md` returns nothing new.
- `/smelt:validate incremental_models` is run for a **drift report read only as input** — this
  phase must not *increase* drift; closing bullets is later phases' work.

## Tasks

1. Read §"The graph layer", §"Delta signatures", §Design of `incremental_models.md` and the
   two divergence bullets this outcome names; confirm the wording deltas above land without
   contradicting §"Per-cell write addressing".
2. Write spec delta 1 (new §Semantics "Dispatch — from propagated components to run units").
3. Write spec delta 2 (keyed dirt-sets carry values; purity via seeds; composition rules).
4. Write spec delta 3 across `run_state.md` + `state.md` inventory + `incremental_models.md`
   §Surface CLI/run-flags wording.
5. Write spec delta 4 (four §Design paragraphs) and delta 5 (reword, don't delete, the stale
   divergence clauses).
6. Cross-check `sources.md` §"Landed-delta (derived, recorded)" and `incremental_shapes.md`
   references still read true against the amended wording; fix pointers only where broken.
7. Run the gates below; write `phases/01-summary.md`.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test state_docs`
- The timeless-oracle grep above.

## Commit message

`docs(incremental): pin scheduler delta-component currency, key-valued dirt, and the per-source watermark`
