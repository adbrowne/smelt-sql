# Master plan: Incremental-model eligibility expansion

**Date**: 2026-07-02
**Design / audit**: `docs/research/20260701-expanding-incremental-eligibility.md`
**Spec**: `docs/specs/incremental_models.md` — the incremental ≡ full-refresh contract and the
Event-time monotonicity trace section (the correctness oracle for every wave here).
**Tracking branch**: `worktree-incremental` (this worktree is the checkout the autonomy loop
drives; running the loop here pushes `worktree-incremental`).

This is the **master plan** for expanding which models smelt can incrementalise, per the
eligibility audit. It carries **no probe rows** — it is a pure feature backlog driven by the
registry below. Read the "## Spawned sub-plans (remediation)" registry, find the first row whose
Status is **not** `done` and whose sub-plan still has a `pending` phase, and run that sub-plan's
next `pending` phase per the sub-plan's own per-phase routine (pre-flight → the phase's spec
increment **only if its row lists one** → `/smelt:implement` red-green with implementer +
reviewer, spec as oracle → verification gates → set the row to `done` → commit + push with the
phase's commit message). If that was the sub-plan's last `pending` phase, flip its registry
Status to `done (<today>)` here and commit together. Emit exactly one sentinel:
`<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>` (record-and-continue), `<<SUBPLAN_ADVANCED>>`, or
`<<MASTER_EXHAUSTED>>`.

**When no registry row is READY** (none is non-`done` with a `pending` phase), emit
`<<MASTER_EXHAUSTED>>` with a one-line summary of which waves remain unscaffolded (see "## Wave
scaffolding queue"). That is the cue for a human to scaffold the next wave's sub-plan, add its
registry row, and re-launch. **Never scaffold a sub-plan or author a new spec/plan
autonomously** — that is the human gate. A sub-plan phase MAY perform a spec increment only when
that phase's row explicitly lists one (pre-authorised by the human who wrote the sub-plan).

## Context

smelt rejects a model from incremental materialisation in many situations; some rejections are
correctness laws, others are conservative mechanical limitations that a well-characterised
sub-case proves safe (audit Parts 2–11). Three of the highest-value relaxations —
`UNION`-branch partitionability, subquery/CTE pushdown, and join driving-fact resolution — all
block on **one** missing analysis: does the projected `event_time` trace back, monotonically, to
a real source partition column (audit Part 6)? So the first wave lands and exhaustively tests
that shared primitive **before** any consumer is wired. Each later wave is a separate,
human-scaffolded sub-plan that consumes the primitive.

## Spawned sub-plans (remediation)

**This registry table is the loop's source of "ready" work.** Each iteration scans it
top-to-bottom; a sub-plan whose Status is **not** `done` and that still has a `pending` phase is
executed before the loop reports `<<MASTER_EXHAUSTED>>`. To queue the next wave, scaffold its
sub-plan and add a NOT-`done` row here.

| Sub-plan | Wave / what it delivers | Status |
|----------|-------------------------|--------|
| `docs/plans/20260702-monotonicity-primitive-tested.md` | W1 — the pure event-time monotonicity trace primitive (`smelt-logical`), then a **generative smelt-sql soundness oracle** (`smelt-db`) that compiles generated models through smelt's own backend codegen and searches input data for any `Traceable` verdict that breaks the commutation contract (DuckDB now, Spark seam), nullability-gated in `smelt-db`, then the spec's open questions resolved from what the tests proved. **Unwired** into any user-visible gate. | done (2026-07-02) |

## Wave scaffolding queue (human-gated — NOT registered until scaffolded)

Scaffold each as its own sub-plan (own spec-diff + docs-site update) and add a registry row when
ready. The W1 primitive's output type is designed for all three consumers.

- **W2 — Consumer A: `UNION`-branch partitionability** (audit §2.5 / E1). Per-branch trace; a
  `StaticSeed` branch is the P3 NULL/constant hazard (named + rejected); all-`Traceable`
  branches unlock Strategy-A wrap-and-filter.
- **W3 — Consumer B: subquery/CTE pushdown conservatism** (audit §4.6 / B4 / E2). Resolve the
  outer `event_time` through the body; `Traceable → source-column` licenses the push,
  `NotTraceable →` stay at the outer clamp. Unifies B4 + E2, closes the CTE bypass (§3.3).
- **W4 — Consumer C: join driving-fact resolution** (audit §5.4). Trace against every join
  input; exactly-one `Traceable` = driving fact; two = multi-clock (J4, reject); zero =
  dim-side clock (reject). Replaces the A5 substring test.
- **W5 — Tree-annotation injection redesign** (audit Part 4). Replace textual
  `inject_time_filter` / `inject_source_filters` with logical/physical-tree annotation consumed
  by `smelt-planner`'s `plan_printer.rs`; consumers annotate the tree with the trace's semantic
  target, the printer emits SQL. Prerequisite context for W2–W4's final injection step.

## Prerequisite

DuckDB dev library must be reachable for the property suite (Phases 2–3): `DUCKDB_LIB_DIR` +
`LD_LIBRARY_PATH` exported into the loop's environment (the loop's stateless iterations do not
set them). When unset, the DuckDB-gated property tests are skipped, degrading to "unit tests
only" rather than failing.
