# Outcome: Ship algebraic maintenance ladder rungs 3-4 (group-rung retraction, bounded-domain multiset)

**Created:** 2026-08-15
**Status:** queued
**Source:** `docs/specs/incremental_shapes.md` "Ladder rungs 3–4 remain specified ahead of this
profile's use of them"; `docs/plans/20260809-keyed-frontier.md` §Scope;
`docs/outcomes/20260809-rung2-state-shapes/outcome.md` §"Out of scope";
`docs/plans/20260704-model-updates.md` rows C3/C4;
`docs/outcomes/20260815-definition-delta-migrate/outcome.md` §"Out of scope"
**Spec anchors:** `docs/specs/incremental_shapes.md`, `docs/specs/model_properties.md`

## The outcome

Rungs 3 and 4 of the algebraic maintenance ladder join rung 2 (decomposed state, shipped by
`rung2-state-shapes`) as real column families with their own state shape, combiner, and
presentation map. Rung 3 (group-rung retraction) gives a retractable contribution a real per-key
fold path — an un-see mechanism for a contribution a full refresh would remove — which is also the
piece `KeyedRetractableContribution` needs to stop refusing every retractable case outright. Rung 4
(the bounded-domain multiset) admits a column family whose per-key state is a bounded multiset
rather than a single monoid element. Both are proven by the generative maintenance-conformance
suite the same way rung 2 already is.

## Success criteria (checkable)

1. Rung 3's state shape and combiner (group-rung, invertible — a contribution can be un-seen) are
   designed and landed in `docs/specs/incremental_shapes.md` §"Decomposed state (rung 2) in keyed
   models" (renamed/extended to cover rungs 2–3, or a new sibling subsection — decide and record).
2. A retractable per-key contribution gets a real fold path; `KeyedRetractableContribution`'s
   refusal narrows to only the case rung 3 genuinely cannot admit (cross-referenced against
   `docs/outcomes/20260815-keyed-open-questions-buildout`, which owns the classifier/diagnostic
   plumbing — this outcome supplies the fold machinery underneath it).
3. Rung 4's bounded-domain multiset state shape and combiner are designed and landed.
4. Rung 4 is implemented: a column family whose per-key state is the bounded multiset, admitted
   under the same admission-matrix discipline as every other family.
5. The generative maintenance-conformance suite exercises rungs 3 and 4 the way it already
   exercises rung 2 (staged history against the full-refresh oracle).
6. `/smelt:validate incremental_shapes` reports no drift for the bullets this outcome closes; the
   "Ladder rungs 3–4" divergence bullet is removed, not merely narrowed. All standing gates green.

## Out of scope

- Rung 3's fold machinery depends on the change-feed consumption design
  (`docs/outcomes/20260815-retraction-and-changefeed` owns retraction/change-feed as a first-class
  delta shape) — sequence this outcome's rung-3 phases after that design lands, or run them in
  parallel with an explicit interface contract agreed up front; don't silently assume the design.
- The `KeyedRetractableContribution` classifier/diagnostic surface itself is owned by
  `docs/outcomes/20260815-keyed-open-questions-buildout`; this outcome supplies the fold path it
  needs, not the diagnostic plumbing.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Design rung 3's state shape + combiner, sequenced against the change-feed design | pending |
| 2 | Implement rung 3's retractable-contribution fold path | pending |
| 3 | Design rung 4's bounded-domain multiset state shape + combiner | pending |
| 4 | Implement rung 4 | pending |
| 5 | Generative conformance suite: rungs 3-4 coverage | pending |
| 6 | Validate + close out: `/smelt:validate incremental_shapes` clean, divergence bullet removed, standing gates green | pending |

## Decision log

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
