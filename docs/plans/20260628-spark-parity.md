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
| `docs/plans/20260628-spark-w1-runtime-harness.md` | W1 — Spark runtime scripts + dual-target test harness + smoke (`examples/multi_engine` on Spark) + **recorded empirical break list** that scopes W2+ | done (2026-06-28) |
| `docs/plans/20260628-spark-w2-unblock-resmoke.md` | W2 — fix the two blocker-class breaks (BL-1 host-path seed load, BL-2 session schema-init ordering) + **re-smoke** to extend the break list that scopes W3 | done (2026-06-28) |
| `docs/plans/20260628-spark-w3-source-seeding.md` | W3 — seed source data identically into both `{DuckDb, Spark}` in the dual-target smoke (resolves BL-3/BL-4, a test-harness fix — **not** a pipeline source-load feature) + **re-smoke** to surface the dialect breaks that scope W4 | pending |

## Wave scaffolding queue

Scaffolded **just-in-time**, one wave at a time, by a human after the prior wave lands. W4+ are
intentionally **not** detailed until W3's re-smoke produces concrete dialect failures — they are
sketches, not commitments. (W3 — source seeding in the smoke + re-smoke — is now scaffolded and in
the registry above.)

- **W4 — dialect lowerings.** One phase per real lowering W3's re-smoke surfaces (QUALIFY →
  subquery, `DATE '…'` → `to_date`, `::` → `CAST`, `[…]` → `ARRAY(…)`, trailing-comma strip, …).
  Each: red dual-target test → printer lowering → green. Oracle: `multi_backend.md` §Semantics
  "Required lowerings". **If the `multi_engine` models are too simple to surface dialect breaks**
  (W3·P3 records a clean parity pass), W4's breaks come from the **broad CLI mirror (W5)** instead —
  scaffold W4 from whichever wave first surfaces a real Spark dialect rejection.
- **W5 — broad CLI mirror.** Parametrize the bulk of the ~50 DuckDB CLI integration tests over
  `{DuckDb, Spark}` via the W1 harness; fix each exec/state gap (incremental DELETE+INSERT,
  schema evolution, seeds, MERGE on Delta).
- **W6 — capability conformance + cross-engine.** One conformance test per `BackendCapabilities`
  flag; assert the constructors match the `multi_backend.md` matrix; validate Spark→DuckDB
  Parquet type conformance (decimal precision, timestamp TZ).
- **W7 — CI gate + docs.** Gated CI job (`spark-up.sh` → `cargo test --features spark` with
  `SPARK_CONNECT_URL` → `spark-down.sh`); `CLAUDE.md` "Commands" entry; `docs-site/` backend
  pages; retract the `multi_backend.md` "parity not yet verified" Known-Divergence once green.

**Deferred product decision (not a wave yet).** W3 surfaced that smelt has **no source-
materialization step in the run pipeline for any backend** — `smelt.sources.*` compiles to a table
name but nothing creates the table; DuckDB examples seed sources manually in `run.sh`. W3 seeds at
the *test* level to unblock parity. Whether smelt should grow a real source-load step (pipeline
pre-load, or declarative per-source `path:`/`data:` with `read_parquet`/`spark.read` lowering) is a
product decision to be taken separately, with a spec, before it becomes a wave.

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
- **2026-06-28** — W1 **fully done** (P2 was committed 2026-06-28; P3 completed this iteration).
  `crates/smelt-cli/tests/spark_smoke.rs` smoke harness committed. Two break-list items recorded:
  **BL-1** (host-temp Parquet path in `load_table` visible to host but not Spark container) and
  **BL-2** (Spark backend does not auto-create the target schema — `[SCHEMA_NOT_FOUND]` on first
  run). W2 scaffolding ready: human should add schema-auto-create fix (BL-2) and Parquet-exchange
  fix (BL-1) as W2 phases and add the W2 registry row.
- **2026-06-28** — **W2 fully done** (P3 re-smoke complete). Two new breaks recorded: **BL-3**
  (source table `analytics.sources_raw_sessions` not materialized in Spark — run pipeline has no
  source-load step + datagen not run) and **BL-4** (cascade: `analytics.staging_stg_sessions`
  absent because BL-3 blocked `stg_sessions`). The smoke ran past session-init (BL-2 resolved) and
  past seed-load (BL-1 resolved), reaching actual SQL execution on both models. Extended break list
  committed. W3 scoped: pre-run source loading + datagen step in the smoke harness.
- **2026-06-28** — **W3 scaffolded** (`docs/plans/20260628-spark-w3-source-seeding.md`, registry row
  added). Investigation of BL-3 found smelt has **no pipeline source-materialization step for any
  backend** (DuckDB examples seed sources manually in `run.sh`); the compiler emits
  `analytics.sources_raw_sessions` but nothing creates it. **Human-gate decision:** keep W3 a
  *test-harness* wave — seed source data identically into both `{DuckDb, Spark}` in the smoke
  (P1 reusable seed helper → P2 wire into the `multi_engine` smoke, resolving BL-3/BL-4 → P3 re-smoke
  to surface dialect breaks) — and treat real source materialization as a separate product decision
  (recorded under the wave queue's "Deferred product decision"). No spec change. Dialect lowerings
  renumber to **W4**, scaffolded from W3's re-smoke breaks.
- **2026-06-28** — **W2 scaffolded** (`docs/plans/20260628-spark-w2-unblock-resmoke.md`, registry
  row added). Reframed from the original "W2 = dialect lowerings" sketch: the recorded break list is
  **shallow because BL-2 aborts every model on first run**, so no dialect break could surface in W1.
  W2 is therefore an **unblock-and-re-smoke** wave — P1 fixes BL-2 (new `requires_schema_init`
  capability flag + create-schema-before-`setCurrentDatabase` ordering; root cause is
  `spark_adapter.py::__init__` selecting the schema before `ensure_schema()` runs), P2 fixes BL-1
  (load Arrow via Connect `createDataFrame`, not a host-path Parquet), P3 re-runs the smoke to
  **extend** the break list with the now-reachable dialect/exec breaks. Dialect lowerings renumber to
  **W3**, scaffolded by a human from W2's extended list. Spec diff landed in `multi_backend.md`
  (matrix row + "Session initialization" / "Loading data into a backend" §Semantics + a data-loading
  §Constraint).
