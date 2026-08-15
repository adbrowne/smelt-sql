# Outcome: Ship contract-lattice v2 (retention, reconciliation points, declared indifference, per-column-group freshness)

**Created:** 2026-08-15
**Status:** queued
**Source:** `docs/specs/incremental_models.md` §Future Extensions ("Lattice v2");
`docs/research/20260811-delta-signatures-and-definition-deltas.md` §6 step 3;
`docs/outcomes/20260815-definition-delta-migrate/outcome.md` §"Out of scope"
**Spec anchors:** `docs/specs/incremental_models.md`

## The outcome

The contract lattice gains its v2 points, built on the v1 default point plus `frozen_horizon` and
`deferral` (`docs/outcomes/20260809-contract-lattice-v1`, done): **retention** (how long a stored
value must survive before it may be reclaimed), **reconciliation points** (declared moments a
relaxed guarantee must resolve back to full equivalence), **declared indifference** (a modeller
stating a column's value doesn't matter past some condition, distinct from `contract: plausible`),
and **per-column-group freshness** (different columns of the same output tolerating different
staleness). Each point is a complete triple — declaration schema, pure oracle transform, probe
emitter — single-owned in `smelt-logical` per the contract-lattice single-ownership rule, so the
conformance gate and runtime probes consume the same definition no lattice point is ever defined
ad hoc by a caller.

## Success criteria (checkable)

1. All four v2 points (retention, reconciliation points, declared indifference,
   per-column-group freshness) are specified in `docs/specs/incremental_models.md` §"The contract
   lattice", each with its declaration surface, its relaxation of the equivalence invariant, and
   its rejected alternatives recorded in §Design.
2. Each point ships as a declaration schema + oracle transform + probe emitter triple in
   `smelt-logical`, matching the single-ownership rule contract-lattice-v1 established.
3. The conformance gate (`maintenance_conformance`) consumes each new point's oracle transform
   directly — no lattice point gets its own ad hoc comparator anywhere in the codebase.
4. Runtime probes for each point emit from the same shared definition the conformance gate uses.
5. docs-site documents all four points with worked examples.
6. `incremental_models.md` §Future Extensions' "Lattice v2" entry is removed (promoted into the
   normative body) and `/smelt:validate incremental_models` reports no drift. All standing gates
   green.

## Out of scope

- Proofs-as-product's guarantee-summary rewrite (`docs/outcomes/20260815-proofs-as-product`) is
  sequenced *after* this outcome specifically because it needs to print whatever v2 adds — this
  outcome does not touch `smelt explain`'s guarantee-summary rendering beyond what a new lattice
  point's probe naturally surfaces via the existing per-cell plan rendering.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec: draft all four v2 lattice points in `incremental_models.md` §"The contract lattice" + §Design | pending |
| 2 | Retention: declaration + oracle transform + probe emitter | pending |
| 3 | Reconciliation points: declaration + oracle transform + probe emitter | pending |
| 4 | Declared indifference: declaration + oracle transform + probe emitter | pending |
| 5 | Per-column-group freshness: declaration + oracle transform + probe emitter | pending |
| 6 | Conformance gate extension for all four points | pending |
| 7 | docs-site: worked examples for each point | pending |
| 8 | Validate + close out: `/smelt:validate incremental_models` clean, Future Extensions entry promoted, standing gates green | pending |

## Decision log

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
