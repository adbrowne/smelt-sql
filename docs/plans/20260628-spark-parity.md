# Master plan: Spark → DuckDB parity

**Date**: 2026-06-28
**Design**: `docs/research/20260628-spark-duckdb-parity.md`
**Spec**: `docs/specs/multi_backend.md` — the parity contract + capability matrix (the
correctness oracle for every wave).
**Tracking branch**: `worktree-spark` (this worktree is the checkout the autonomy loop drives;
running the loop here pushes `worktree-spark`).

This is the **master plan** for bringing the Spark backend to verified parity with DuckDB. It
carries **no probe rows** — it is a pure feature backlog driven by the registry below. Read the
"## Spawned sub-plans" registry, find the first row whose Status is **not** `done` and whose
sub-plan has a `pending` phase, and run that sub-plan's next `pending` phase per the sub-plan's
own per-phase routine. If that was the sub-plan's last `pending` phase, flip its registry
Status to `done (<today>)` here and commit together. Emit exactly one sentinel:
`<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>` (record-and-continue), `<<SUBPLAN_ADVANCED>>`, or
`<<MASTER_EXHAUSTED>>`.

**When no registry row is READY** (none is non-`done` with a `pending` phase), emit
`<<MASTER_EXHAUSTED>>` with a one-line summary of which waves remain unscaffolded (see "## Wave
scaffolding queue"). That is the cue for a human to scaffold the next wave's sub-plan, add its
registry row, and re-launch. **Never scaffold a sub-plan or edit a spec autonomously** — that
is the human gate. In particular, W2+ ("fix the gaps") is scaffolded by a human **from W1's
recorded break list**, not invented by the loop.

## Prerequisite (human, one-time — must be live before the loop runs Spark tests)

A Spark Connect server must be **running and reachable** at a stable `SPARK_CONNECT_URL`
before any Spark-targeted phase runs. The loop's iterations are stateless `claude --print`
processes; they do **not** stand Spark up. Bring it up once with `scripts/spark-up.sh` (created
in W1·P1) and export `SPARK_CONNECT_URL` into the loop's environment. If `SPARK_CONNECT_URL` is
unset when a Spark phase runs, its Spark assertions **skip** (green) rather than fail — so a
mis-provisioned host degrades to "harness built, Spark coverage skipped", which a phase records
rather than blocking on.

## Context

smelt already ships a substantial but **largely unverified** Spark backend (see the design doc
for the full inventory). Parity here means: a dual-target test matrix proves the same smelt
model behaves the same on Spark as on DuckDB, and each real gap the matrix surfaces is fixed by
a dialect/exec lowering. The capability matrix in `docs/specs/multi_backend.md` is the oracle.

## Spawned sub-plans

**This registry table is the loop's source of "ready" work.** Each iteration scans it
top-to-bottom; a sub-plan whose Status is **not** `done` and that still has a `pending` phase is
executed before the loop reports `<<MASTER_EXHAUSTED>>`. To queue the next wave, scaffold its
sub-plan and add a NOT-`done` row here.

| Sub-plan | Wave / what it delivers | Status |
|----------|-------------------------|--------|
| `docs/plans/20260628-spark-w1-runtime-harness.md` | W1 — Spark runtime scripts + dual-target test harness + smoke (`examples/multi_engine` on Spark) + **recorded empirical break list** that scopes W2+ | pending |

## Wave scaffolding queue

Scaffolded **just-in-time**, one wave at a time, by a human after the prior wave lands. W2+ are
intentionally **not** detailed until W1 produces concrete failures — they are sketches, not
commitments:

- **W2 — dialect lowerings.** One phase per real lowering W1's break list surfaces (QUALIFY →
  subquery, `DATE '…'` → `to_date`, `::` → `CAST`, `[…]` → `ARRAY(…)`, trailing-comma strip,
  …). Each: red dual-target test → printer lowering → green. Oracle: `multi_backend.md`
  §Semantics "Required lowerings".
- **W3 — broad CLI mirror.** Parametrize the bulk of the ~50 DuckDB CLI integration tests over
  `{DuckDb, Spark}` via the W1 harness; fix each exec/state gap (incremental DELETE+INSERT,
  schema evolution, seeds, MERGE on Delta).
- **W4 — capability conformance + cross-engine.** One conformance test per `BackendCapabilities`
  flag; assert the constructors match the `multi_backend.md` matrix; validate Spark→DuckDB
  Parquet type conformance (decimal precision, timestamp TZ).
- **W5 — CI gate + docs.** Gated CI job (`spark-up.sh` → `cargo test --features spark` with
  `SPARK_CONNECT_URL` → `spark-down.sh`); `CLAUDE.md` "Commands" entry; `docs-site/` backend
  pages; retract the `multi_backend.md` "parity not yet verified" Known-Divergence once green.

## Blocked phases (master-level triage ledger)

Append-only log of phases the loop recorded as `blocked` in a sub-plan that need human triage at
the master level.

_(none yet)_

## Status

- **2026-06-28** — Master + W1 scaffolded; `multi_backend.md` spec authored; design doc
  committed. W2+ await W1's break list.
- **2026-06-28** — W1·P1 **done** interactively: Spark Connect (Spark 4.1.1, image
  `apache/spark:latest`) live on `:15002` via `scripts/spark-up.sh`; pinned pyspark 4.1.1 client
  venv; all 8 `smelt-backend-spark` integration tests green (incl. MERGE). First break-list item
  recorded — **BL-1**: the seed-load Parquet exchange assumes Spark shares the host filesystem
  (breaks for containerized/remote Connect). P2 (harness) + P3 (smoke) remain.
