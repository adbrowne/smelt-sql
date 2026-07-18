# Plan: Quality Grind — deferred small fixes, conformance gaps, generator coverage (MASTER)

**Date**: 2026-07-18
**Spec**: [`docs/specs/architecture.md`](../specs/architecture.md) §"Constraints & Invariants" items 13–14 (dialect-conformance + function-registry gates), [`docs/specs/diagnostics.md`](../specs/diagnostics.md)
**Spec diff**: none — this programme drives the implementation toward the *existing* gate contracts (ledger ratchet, registry gates, proptest oracles). No spec change is required; any phase that discovers a needed spec change must block and surface it.
**Tracking PR / branch**: `worktree-roadmap_todo`
**Docs**: code+docs

---

## What this master is

A registry-driven backlog (no probe rows) of small-to-medium quality items harvested from
`docs/TODO.md` and `docs/ROADMAP.md` §"Deferred-Work Backlog" on 2026-07-18. Every item
has (a) a standing CI gate or property oracle as its correctness anchor and (b) a
root-cause note from a prior triage — the loop should never need to make a design
decision. Anything that turns out to need one is marked `blocked` per the block rule,
never improvised.

## Iteration routine (for the loop)

1. Read the "## Spawned sub-plans" registry below, top-to-bottom. A sub-plan is **READY**
   when its registry Status is **not** `done` **and** its own Progress-tracking table has
   at least one `pending` row. The first READY sub-plan is this iteration's target.
2. Execute the next `pending` phase of that sub-plan following its own per-phase routine
   (pre-flight → red-green `/smelt:implement` with implementer + reviewer → verification
   gates → set the row `done` → commit + push). If that was the sub-plan's last `pending`
   phase, also flip its registry Status to `done (<today>)`.
   Emit `<<PHASE_COMPLETE>>` (or `<<PHASE_BLOCKED>>` per the block rule).
3. If **no** sub-plan is READY, emit `<<MASTER_EXHAUSTED>>` with a one-line summary — a
   human then triages the "Tier 3 — decision queue" below and scaffolds the next
   sub-plan. **The loop never scaffolds a sub-plan or authors a spec autonomously.**

## Spawned sub-plans

**This registry table is the loop's source of "ready" work.** Each iteration scans it
top-to-bottom; the first sub-plan whose Status is **not** `done` and that still has a
`pending` phase is executed.

| Sub-plan | What it delivers | Status |
|----------|------------------|--------|
| [`docs/plans/20260718-quality-grind-t1.md`](20260718-quality-grind-t1.md) | **Tier 1 — small, root-caused fixes.** Parser ledger categories triaged "Small" (`NOT`-prefixed binary operators, `==`, quoted table names in FROM, `RANGE` as identifier/function, `NULL::TYPE`, parenthesized set-op trailing ORDER BY), the TABLESAMPLE/alias ordering bug, VALUES-body CTE arity, sub-day interval mis-parse, UTF-8 diagnostic positions + smelt-ui LineIndex, registry gaps (`to_seconds`, `md5`), and the documentation-gap batch. | pending |
| [`docs/plans/20260718-quality-grind-t2.md`](20260718-quality-grind-t2.md) | **Tier 2 — well-understood, larger.** Property-test generator deferred items (aggregate FILTER, ordered-set aggregates / WITHIN GROUP, two-column aggregates, ARRAY, ROW/STRUCT), the smelt-planner↔smelt-logical duplicated-module consolidation, the cold-Salsa 2000-model benchmark regression investigation, and two CLI ergonomics fixes. | pending |

## Progress tracking (human-facing overview)

| Tier | Sub-plan | Phases | Status |
|------|----------|--------|--------|
| 1 | 20260718-quality-grind-t1.md | 12 | pending |
| 2 | 20260718-quality-grind-t2.md | 9 | pending |

## Tier 3 — decision queue (NOT loop work)

Items from the same harvest that are gated on a human design decision. The loop must
never pick these up. Each will be either ratified into a future sub-plan here or
explicitly parked. Decisions recorded inline as they are made.

| # | Item | Decision needed | Status |
|---|------|-----------------|--------|
| D-QG-1 | P7c Map-loader direction (`docs/TODO.md` §P7c, paused 2026-06-03) | Pick (A) drop Map-in-model, (B) wire Map consumption (`keys()`/`entries()`), or (C) exempt bare loaders | open |
| D-QG-2 | Implicit comma-join semantics (25 ledger entries) | Whether `FROM a, b` becomes a first-class cross-join in `JoinClause::join_type()` (inference semantics, not grammar) | open |
| D-QG-3 | ON-join `SELECT *` right-side expansion (`docs/TODO.md` 2026-07-12) | Admitting duplicate column names into inferred schemas — affects find-by-name consumers, LSP completion, input-constraint keying | open |
| D-QG-4 | `smelt bakeoff` CLI (ROADMAP §10, deferred) | Three design questions: single-technique force-execute plumbing, `--pin` frontmatter round-trip, `smelt-cli` commands-module visibility | open |
| D-QG-5 | Cold-Salsa benchmark ceiling (fed by T2 Phase 8's profile) | Optimize the offending analysis vs raise the 10s ceiling | open — awaiting T2·P8 findings |

## Deferred during implementation

(Append-only. Items surfaced during the work that we chose not to handle in this programme.)

## Verification

- Both sub-plans' own Verification sections green.
- `bash .claude/scripts/verify-phase.sh` on the final tree.
- Ledger/ratchet deltas recorded per phase: `.claude/parser-gaps-baseline.txt` and the
  external-ledger entry count only ever shrink; `.claude/registry-migration-baseline.txt`
  unchanged or shrunk.
