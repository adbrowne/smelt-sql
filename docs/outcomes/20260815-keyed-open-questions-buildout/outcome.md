# Outcome: Build out the key grain's remaining "(Open Question)" bullets

**Created:** 2026-08-15
**Status:** queued
**Source:** `docs/specs/incremental_shapes.md` §"The key grain" §Known Divergences / Open Questions;
`docs/outcomes/20260815-definition-delta-migrate/outcome.md` §"Out of scope" (the
"genuinely large product calls" bucket, moved here per the 2026-08-15 build-everything decision)
**Spec anchors:** `docs/specs/incremental_shapes.md`, `docs/specs/incremental_models.md`

## The outcome

Every `(Open Question)` bullet the key grain's §Known Divergences still carries gets a real
decision *and* a shipped implementation, not just prose: `g_run >= g_part` auto-coarsening (or a
decided reject-with-suggestion form) is built; snapshot-reconcile admits a proven multi-source
scan instead of refusing at two-or-more unclocked candidates; `KeyedRetractableContribution` gets
a real classifier, diagnostic, and test; a re-run-tolerant (non-additive) model gets a ledger too,
and the reconciliation ledger's fold becomes transactional on every backend, not only DuckDB; the
once-write nullability route admits a provably key-derived *expression*, not only a bare
`unique_key` column reference; the pattern functions (`smelt.latest`/`smelt.once`/`smelt.current`)
ship, decided as built-ins or a shipped `smelt.define` template; driver granularity widens below
`day`/`week`; `NOW()`/`CURRENT_*` in keyed models get the same compile-time pinning the partition
grain already has instead of an outright rejection; self-referential keyed models are admitted via
an explicit input/state distinction design. Two related residues close alongside these: the
derived execution postures (order-independence specifically) become a real derived verdict printed
by `smelt explain`, not merely internal; and the generative maintenance-conformance pool gains a
nullable payload type so the once-write family's NULL direction is proven by the generated pool
like every other keyed family, not by one hand-written test case.

## Success criteria (checkable)

1. `g_run >= g_part` sub-granularity run windows either auto-coarsen or reject with a concrete
   suggested window — the decision is recorded in `incremental_shapes.md` §Design and the "(Open
   Question)" tag is dropped.
2. Snapshot-reconcile admits two or more unclocked FROM-clause candidates via a proven
   multi-source scan; `KeyedSnapshotPostureUnsupported` narrows to the residual genuinely
   ambiguous case.
3. `KeyedRetractableContribution` has a real classifier, a fixture that produces it, and a test —
   the "no implementation" divergence is gone.
4. A re-run-tolerant (non-additive) window-forward model writes a frontier record too; `--auto`
   staleness can consult it. The reconciliation ledger's fold is transactional on every shipped
   backend, not DuckDB-only.
5. The once-write nullability route admits a provably key-derived expression (not only a bare
   `unique_key` column), closing the fallback-case gap named in `incremental_shapes.md`.
6. `smelt.latest`/`smelt.once`/`smelt.current` ship (decision recorded: built-in vs. template) and
   are reachable without hand-written SQL spellings.
7. Driver granularity supports at least one unit finer than `day`; `NOW()`/`CURRENT_*` in keyed
   models are compile-time pinned per run instead of refused; self-referential keyed models
   (`state += delta − decay`) are admitted under an explicit input/state distinction.
8. Order-independence (and the other derived execution postures) is computed as a named verdict,
   not merely assumed sequential, and `smelt explain` prints it alongside the run shape.
9. The generative conformance pool's row type carries a nullable payload; the once-write NULL
   direction is covered by the generated pool for every keyed family it applies to.
10. `/smelt:validate incremental_shapes` reports no drift for every bullet this outcome closes;
    all standing gates green.

## Out of scope

- `--auto` staleness fidelity beyond conservative v1 needs the group rung's delta-history
  mechanism — depends on `docs/outcomes/20260815-ladder-rungs-3-4` landing rung 3 first. Sequence
  this outcome's phase for that item after rung 3, or split it out if the dependency blocks too
  long; do not silently drop it.
- Building the observer/prefix-consistency contract for non-replayable combinations —
  `docs/outcomes/20260815-scd2-watermark-observer-contract` owns that.

## Phases

| # | Phase | Status |
|---|-------|--------|
| 1 | `g_run >= g_part`: decide auto-coarsening vs. reject-with-suggestion, implement, record the decision | pending |
| 2 | Snapshot-reconcile: admit a proven multi-unclocked-source scan | pending |
| 3 | `KeyedRetractableContribution`: classifier, diagnostic, fixture, test | pending |
| 4 | Ledger presence for re-run-tolerant models; transactional ledger fold on every backend | pending |
| 5 | Once-write nullability route for a key-derived expression | pending |
| 6 | Ship the pattern functions (decide built-in vs. template) | pending |
| 7 | Driver granularity below `day`/`week`; run-pinning alignment; self-referential keyed models | pending |
| 8 | Derive and print execution postures (order-independence) in `smelt explain` | pending |
| 9 | Generative conformance pool: nullable payload, once-write NULL direction covered | pending |
| 10 | Validate + close out: `/smelt:validate incremental_shapes` clean, standing gates green | pending |

## Decision log

## Blocked

<!-- Dated entries: phase, reason, candidate options. -->
