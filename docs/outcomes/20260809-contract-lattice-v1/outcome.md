# Outcome: Contract lattice v1 — frozen horizons and deferral

**Created:** 2026-08-09
**Status:** queued
**Source:** `docs/research/20260809-incremental-rethink.md` §4.2, §6 step 5; `docs/research/20260726-beyond-ivm-differentiation.md` §5.2/§5.3
**Spec anchors:** `docs/specs/incremental_models.md` (the equivalence invariant)

## The outcome

The equivalence invariant becomes the *default* point in a small declared
contract lattice. v1 ships the two relaxations with the clearest oracles:
**frozen horizon** (partitions older than H are never revisited; late data
outside H is diagnosed, not silently excluded — closing the one accepted
silent-data behaviour in the family) and **deferral** (a cell may lag its
inputs by up to D, licensing run skipping and work subsumption). Each
relaxation is declared, validated, probe-checked, and printed by
`smelt explain`; the conformance oracle is parameterised per lattice point.

## Success criteria (checkable)

1. `frozen_horizon:` declared on a partition-grain model: writes outside H
   are clamped by contract (not merely by derived reach), and a genuinely
   late arrival outside H raises a named diagnostic instead of being silently
   excluded (deleting that Known Divergence).
2. `deferral:` declared on a cell/model: a run whose pending input set is
   within the deferral window may be skipped, and a pending small run implied
   by a scheduled larger one is subsumed (the ledger proves the subsumption).
3. The spec defines the lattice: the default contract, the two v1 points,
   each with its restated oracle and its probe; declarations compose with the
   existing shape facts without new modes.
4. `smelt explain` prints the effective contract per cell — default or
   relaxed, with the relaxation's parameters.
5. `maintenance_conformance` is parameterised by lattice point: relaxed cells
   are asserted against their *relaxed* oracle, default cells against strict
   equivalence; a relaxation is never silently tested as the default.
6. All standing gates green.

## Out of scope

- Other lattice points (reconciliation points, declared indifference,
  per-column-group freshness, retention) — v2+ once these two prove the shape.
- Restating the invariant per-cell in the spec headline (that is the spec
  redraft outcome's job; v1 adds the lattice without rewriting the whole spec).

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec: the lattice framing — default point, frozen horizon, deferral; oracles and probes per point | pending |
| 2 | `frozen_horizon:` declaration, validation, write-eligibility clamp wiring | pending |
| 3 | Late-arrival diagnostic outside the frozen horizon (delete the silent exclusion) | pending |
| 4 | `deferral:` declaration + run skipping; ledger-proven work subsumption | pending |
| 5 | Conformance oracle parameterised per lattice point + recipes for both relaxations | pending |
| 6 | Surface: explain contract rendering, docs-site update | pending |

## Decision log

<!-- Dated one-liners appended by plan/implement steps. -->

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
