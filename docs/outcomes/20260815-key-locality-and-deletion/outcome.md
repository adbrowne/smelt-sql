# Outcome: Finish key temporal locality and give keyed models a real deletion mechanism

**Created:** 2026-08-15
**Status:** queued
**Source:** `docs/specs/incremental_shapes.md` §"Key temporal locality (the time-partitioned
output)" §Known Divergences and the Locality/key-deletion Open Questions;
`docs/research/20260705-keyed-time-superset.md`,
`docs/research/20260705-keyed-collapse-application.md` §5;
`docs/plans/20260705-keyed-collapse.md`, `docs/plans/20260715-composed-axes-conditional-maintenance.md`;
`docs/outcomes/20260815-definition-delta-migrate/outcome.md` §"Out of scope"
**Spec anchors:** `docs/specs/incremental_shapes.md`

## The outcome

Key temporal locality's route 2 (key-determined) admits a provably key-derived expression, not
only a column covered by a declared `functional_dependencies:` entry — the sub-route the spec
already names but the classifier never reaches. The per-input scope-map explain surface ships.
Route 3's `IN (SELECT DISTINCT …)` slice predicate gets a real backend-exercised spelling (working
around, or confirming and documenting, the DuckDB MERGE binder limitation). Granularity
determination and declared-vs-derived recurrence precedence — both underdetermined by the spec
text today — are decided and implemented consistently. Locality slice pruning extends to
snapshot-reconcile wherever a bound is provable, and the granularity-equality precondition relaxes
to admit a coarser output than its driving source where safe. Slice-scoped deletion ships. Key
deletion moves past pure retention: a real mechanism (tombstones and/or opt-in hard delete) exists
for a departed key, with the observer contract stated for the currently-refused snapshot matrix
cells. `key_per_partition` — today a derived label that refuses at plan derivation with
`MaintenanceUnsupportedGrain` — gets a real maintenance-plan derivation (trajectory support).

## Success criteria (checkable)

1. Route 2 admits a provably key-derived expression in addition to the declared-FD route; the
   per-input scope-map is rendered by `smelt explain`.
2. Route 3's `IN (SELECT DISTINCT …)` slice predicate runs against a real backend end to end (a
   working spelling, or a documented capability-struct-declared alternate lowering if the DuckDB
   binder limitation is confirmed unworkaroundable).
3. Granularity determination and declared-vs-derived recurrence precedence are decided, recorded
   in `incremental_shapes.md` §Design, and implemented consistently across all three routes.
4. Slice pruning is admitted under snapshot-reconcile wherever a bound is provable (v1's
   window-forward-only restriction is lifted or explicitly narrowed with a stated reason).
5. The granularity-equality precondition relaxes to admit a coarser keyed-output granularity than
   its driving source where the relation is provable.
6. Slice-scoped deletion ships.
7. Key deletion beyond retention has a real mechanism (tombstones and/or opt-in hard delete); the
   observer contract for the currently-refused snapshot-observer-semantics matrix cells is stated
   and, where the mechanism now makes them safe, the refusal narrows.
8. `key_per_partition` derives a real maintenance plan (trajectory-aware) instead of refusing
   `MaintenanceUnsupportedGrain` at plan derivation.
9. `/smelt:validate incremental_shapes` reports no drift for every bullet this outcome closes; all
   standing gates green.

## Out of scope

- None. Both cited tracking plans (`20260705-keyed-collapse`, `20260715-composed-axes-conditional-maintenance`)
  predate `docs/outcomes/`; audit their actual status in phase 1 rather than assuming either is
  still live, per the same discipline `20260815-partition-grain-residue` applies.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | Audit the two cited pre-outcome tracking plans against current repo state | pending |
| 2 | Route 2: admit a provably key-derived expression; per-input scope-map explain surface | pending |
| 3 | Route 3: real backend-exercised `IN (SELECT DISTINCT …)` slice predicate; decide + implement granularity-determination and recurrence precedence | pending |
| 4 | Slice pruning under snapshot-reconcile where provable | pending |
| 5 | Relax the granularity-equality precondition | pending |
| 6 | Slice-scoped deletion | pending |
| 7 | Key deletion beyond retention: tombstones / opt-in hard delete; observer contract for the refused matrix cells | pending |
| 8 | `key_per_partition`: real maintenance-plan derivation (trajectory support) | pending |
| 9 | Validate + close out: `/smelt:validate incremental_shapes` clean, standing gates green | pending |

## Decision log

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
