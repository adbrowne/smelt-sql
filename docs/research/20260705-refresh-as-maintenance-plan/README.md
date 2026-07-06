# Refresh as a maintenance plan — research directory

- **Date**: 2026-07-05 (paper), 2026-07-06 (spec-readiness expansion)
- **Status**: research (design exploration; predecessor to a spec change)
- **Author**: Andrew (with Claude)

This directory is the finished form of the research programme that began as the single-file
paper `20260705-refresh-as-maintenance-plan.md` (now `01-framework.md` here) and was then
empirically stress-tested by the property-discovery loop
(`../20260705-property-discovery-loop.md`, artifacts in `../property-discovery/`). The goal of
the 2026-07-06 expansion: get the framework close enough to done that a full spec can be
written from it.

## The thesis, in one paragraph

A model's incremental maintenance is a plan indexed by `(output-column-group × input-delta)`.
Each cell picks a corner of a 2×2 of *read scope* × *write scope*; recompute-a-region and
fold-a-delta are two of its four corners. Whether two techniques may serve the same cell
interchangeably is governed by one theorem: at a fixed processed-input set they must produce
identical state on the columns that decide which rows exist. Under this lens today's refresh
"modes" are lossy projections of common plans; the *strategy content* of the refresh enum
becomes derived, while output shape/grain stays declared-and-checked.

## Reading order

| # | file | what it answers | length |
|---|---|---|---|
| 1 | [`01-framework.md`](01-framework.md) | The framework itself: the 2×2 technique space, the interchangeability theorem, per-cell plan factoring, skeleton/payload, the generalized ledger, declared-vs-derived. | ~760 |
| 2 | [`02-loop-findings.md`](02-loop-findings.md) | What the property-discovery loop empirically established about smelt (23 cells): execution is unconditionally recompute-region; the universal backfill-recovers trade; dormant classifiers; analyzer soundness; coverage honesty. | ~360 |
| 3 | [`03-design-forks.md`](03-design-forks.md) | Recommended resolutions (awaiting ratification) for the five forks/bugs the loop parked: G-11 clamp wrap, G-10 composite keys, FIX-2 classifier wiring, the BigInt truncation bug, the same-named-column clamp. | ~310 |
| 4 | [`04-knobs.md`](04-knobs.md) | The user-facing configuration surface: refresh trichotomy + declared grain, per-cell `maintenance:` overrides, per-column contracts, backfill/cascade policy, the bake-off CLI. | ~310 |
| 5 | [`05-source-properties.md`](05-source-properties.md) | Source declarations that license techniques: the structured `mutation_profile` block, lateness/watermarks, composite unique keys, clocks, retention, delta identity — each with a verification (tripwire) story. | ~280 |
| 6 | [`06-proof-obligations.md`](06-proof-obligations.md) | The sets of things that must be proven: per-cell admission, skeleton/payload, ledger, source tripwires, cross-technique equivalence (the loop's forward backlog), execution parity — with mechanism/time/failure-mode for each. | ~390 |
| 7 | [`07-example-catalogue.md`](07-example-catalogue.md) | 40 worked examples across constructs × source properties × output shapes × techniques, each with a real-world use case and a machine-scannable header; 19 lift-ready probe cells. Includes Family G — schema evolution (single-field backfill as the 2×2's left column, through the co-sensitive ledger catch-up). | ~990 |
| 8 | [`08-code-placement.md`](08-code-placement.md) | Where it lives: `MaintenancePlan` as pure data in `smelt-logical`, Salsa query in `smelt-db`, choice in `smelt-planner`, lowering in `smelt-runtime`, primitives in `smelt-backend*`, ledger in `smelt-state`; migration sketch M0–M6. | ~320 |
| 9 | [`09-spec-readiness.md`](09-spec-readiness.md) | The gap list: decisions to ratify, machinery that doesn't exist yet, empirical gaps, the spec-diff map, and the definition of "ready to spec". | — |
| 10 | [`10-dependency-propagation.md`](10-dependency-propagation.md) | The graph layer: forward propagation (what landed → which partitions of which models run, per-edge trigger cells) and backward resolution (build a model for a period *including upstreams* — test/validation builds); scenario catalogue S1–S12, granularity/self-reference/column-scoped-dirt roadmap. | ~300 |

Skimming for a decision? Read the Summary of `01`, then `03` (the ratification queue), then `09`
(everything else that stands between here and a spec). The catalogue (`07`) is the reference you
dip into per shape, not a cover-to-cover read.

## Relationship to the property-discovery loop

The loop (`../20260705-property-discovery-loop.md`) is the empirical engine; its artifacts —
[`../property-discovery/ledger.md`](../property-discovery/ledger.md) (per-cell verdicts),
[`../property-discovery/unsupported.md`](../property-discovery/unsupported.md) (negative
catalogue), [`../property-discovery/catalog.md`](../property-discovery/catalog.md) (cell
backlog) — are cited throughout by cell id (`SC-1`, `G-08`, `FIX-1`, …). `02-loop-findings.md`
synthesizes them; `06-proof-obligations.md` §5 and `07-example-catalogue.md`'s closing list give
the loop its next backlog.
