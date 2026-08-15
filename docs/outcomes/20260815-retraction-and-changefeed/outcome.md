# Outcome: Retraction handling and change-feed consumption as a first-class delta shape

**Created:** 2026-08-15
**Status:** queued
**Source:** `docs/specs/definition_deltas.md` §"What stays data-side" and §Future Extensions
(eclipse-detection breadth; row-local derivation for mid-catch-up groups);
`docs/outcomes/20260815-definition-delta-migrate/outcome.md` §"Out of scope" (phase 18's
`change_feed`/`UpstreamMutation` admission is the small decision this outcome builds real fold
machinery on top of)
**Spec anchors:** `docs/specs/definition_deltas.md`, `docs/specs/incremental_models.md`,
`docs/specs/incremental_shapes.md`

## The outcome

Retraction — un-seeing a contribution a full refresh would remove — and change-feed consumption
become a first-class delta shape rather than data-side folklore `definition_deltas.md` explicitly
declines to own today. `change_feed` sources get real fold machinery consuming their delta shape
(insert/update/delete rows), not only the `UpstreamMutation` cell admission
`20260815-definition-delta-migrate` phase 18 already gives them for consistency. This is the design
`docs/outcomes/20260815-ladder-rungs-3-4`'s rung 3 (group-rung retraction) needs underneath it.
Eclipse-detection breadth widens to algebraic identities and join reorderings in the
definition-delta classifier, and row-local derivation for mid-catch-up groups ships, closing both
`definition_deltas.md` §Future Extensions entries.

## Success criteria (checkable)

1. Change-feed sources are specified as a first-class delta shape in `definition_deltas.md` and/or
   `incremental_models.md` — the retraction algebra, its column-family admission rule, and its
   relationship to the algebraic maintenance ladder's rungs.
2. Retraction fold machinery is implemented: a `change_feed` source's delete/retract rows are
   consumed and folded, not just re-triggering full-input re-derivation.
3. `KeyedRetractableContribution`'s repair-family interaction (`incremental_shapes.md`
   §"Enrichment joins") is re-examined against the new fold machinery and updated if it changes
   admission.
4. Eclipse-detection widens to algebraic identities and join reorderings in the definition-delta
   classifier.
5. Row-local derivation for mid-catch-up groups ships.
6. Both `definition_deltas.md` §Future Extensions entries this outcome targets are removed
   (promoted into the normative body); `/smelt:validate definition_deltas` reports no drift. All
   standing gates green, including a new generative-conformance leg for retraction.

## Out of scope

- The rung-3 state shape and combiner themselves belong to
  `docs/outcomes/20260815-ladder-rungs-3-4` — this outcome supplies the change-feed delta-shape
  design and fold machinery that rung 3 consumes, sequenced before or alongside it by explicit
  interface agreement (see that outcome's §"Out of scope").

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Spec: change-feed as a first-class delta shape — retraction algebra, column-family admission, ladder relationship | pending |
| 2 | Retraction fold machinery: implementation | pending |
| 3 | `change_feed` source consumption beyond full-input re-derivation | pending |
| 4 | Re-examine `KeyedRetractableContribution`'s enrichment-join interaction against the new machinery | pending |
| 5 | Eclipse-detection breadth: algebraic identities, join reorderings | pending |
| 6 | Row-local derivation for mid-catch-up groups | pending |
| 7 | Generative conformance leg for retraction; docs-site | pending |
| 8 | Validate + close out: `/smelt:validate definition_deltas` clean, standing gates green | pending |

## Decision log

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
