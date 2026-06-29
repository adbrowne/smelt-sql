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
| `docs/plans/20260628-spark-w3-source-seeding.md` | W3 — seed source data identically into both `{DuckDb, Spark}` in the dual-target smoke (resolves BL-3/BL-4, a test-harness fix — **not** a pipeline source-load feature) + **re-smoke** to surface the dialect breaks that scope W4 | done (2026-06-29) |
| `docs/plans/20260629-spark-w4-shared-backend-factory.md` | W4 — extract one shared `smelt-backends` factory consumed by **both** CLI and UI (closes the CLI↔UI parity gap where the UI's factory was DuckDB-only → **UI gains Spark**), guard test against re-duplication, + fail-loud on unknown backend `type:` | pending |

## Wave scaffolding queue

Scaffolded **just-in-time**, one wave at a time, by a human after the prior wave lands. W5+ are
intentionally **not** detailed until they're reached — they are sketches, not commitments. (W4 —
the shared backend factory — is now scaffolded and in the registry above.)

**Note — the "dialect lowerings" wave dissolved.** An audit (2026-06-29) found **all six**
spec-required Spark lowerings (QUALIFY, `DATE '…'`, `::`, `[…]`, trailing commas, CREATE OR REPLACE
emulation) are **already implemented and unit-tested** in `crates/smelt-dialect/src/printer.rs` (+
the Spark backend). Combined with W3·P3's clean `multi_engine` parity pass, there is **no
dialect-lowering work to scaffold**. The residual risk is that those lowerings are unit-tested with a
synthetic `spark_ctx()` but **not yet executed on live Spark** — so live-execution coverage of them
folds into **W5** (below), not a separate wave.

- **W5 — broad CLI mirror / independent Spark coverage.** Parametrize the bulk of the ~50 DuckDB CLI
  integration tests over `{DuckDb, Spark}` via the W1 harness **and/or** add independent Spark
  integration tests — the goal is *robust Spark coverage*, not DuckDB-test-for-DuckDB-test
  duplication. Fix each exec/state gap surfaced (incremental DELETE+INSERT, schema evolution,
  MERGE on Delta). Two named first-class items:
  - **Seed / `load_table` parity on live Spark.** `Backend::load_table(...)` is smelt's only
    backend-owned ingest path (`docs/specs/seeds.md`); it is what **BL-1** broke (host-temp Parquet
    exchange invisible to a remote Connect JVM, "fixed" in W2·P2 by loading Arrow via Connect
    `createDataFrame`). W5 must prove `smelt seed` / `smelt build` actually loads a CSV seed into a
    live Spark backend end-to-end (red dual-target seed test → green), not just at the W3 harness
    level. Sources are **out of scope** — smelt never loads them (`docs/specs/sources.md`).
  - **Dialect lowerings executed on live Spark.** The six implemented lowerings are unit-tested with a
    synthetic `spark_ctx()` but not yet run against a real server; if any is rejected, fix it here
    (red dual-target test → printer fix → green; oracle `multi_backend.md` §"Required lowerings").
- **W6 — capability conformance + cross-engine.** One conformance test per `BackendCapabilities`
  flag; assert the constructors match the `multi_backend.md` matrix; validate Spark→DuckDB
  Parquet type conformance (decimal precision, timestamp TZ).
- **W7 — CI gate + docs.** Gated CI job (`spark-up.sh` → `cargo test --features spark` with
  `SPARK_CONNECT_URL` → `spark-down.sh`); `CLAUDE.md` "Commands" entry; `docs-site/` backend
  pages (incl. a backend-support note now that the UI runs Spark); retract the `multi_backend.md`
  "parity not yet verified" Known-Divergence once green.

**Not a wave — "source materialization" was a misframing (struck 2026-06-29).** An earlier note
here proposed a future wave to let smelt *materialize sources*. That contradicts `docs/specs/sources.md`,
which is normative: a source is "an external table that already exists in the target database,
populated by some pipeline outside smelt … it never runs `CREATE TABLE` or `INSERT` for the source."
Smelt must **never** load a source — there is no product feature to design here. Untangling what W3
conflated:
- **Sources** — external, never materialized by smelt. Not work. (Spec already forbids it.)
- **Seeds** — the *real* ingest path smelt owns: a CSV (+ optional sidecar) loaded into the backend
  via `Backend::load_table(...)` on `smelt seed` / `smelt build` (`docs/specs/seeds.md`). Making this
  work on Spark **is** genuine parity work — it is exactly the `load_table` path that **BL-1** broke
  (host-temp Parquet exchange invisible to a remote Connect JVM). This belongs in **W5** (live
  `load_table`/seed parity), not a separate "source-load" wave.
- **Populating source tables for tests** — a pure test-harness concern. The dual-target smoke needs
  the source *tables* to exist so models can run; W3's seed helper writes the same rows into both
  `{DuckDb, Spark}` as a fixture. Done in W3; stays at the test level — not a product feature.

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
- **2026-06-29** — **W3 fully done** (P1 seed helper + P2 smoke wiring + P3 re-smoke complete).
  BL-3/BL-4 resolved. **W3·P3 PARITY PASS**: `staging.stg_sessions` and
  `intermediate.int_visitor_daily` both passed on live Spark (Spark 4.1.1) with no dialect errors.
  Break list: **(none)** — `examples/multi_engine` too simple to surface dialect breaks. **W4
  dialect lowerings** will be scoped from **W5 (broad CLI mirror)**, not this re-smoke. Human
  should scaffold W5 (broad CLI mirror over ~50 DuckDB integration tests) next; W4 follows from
  whatever Spark dialect rejections W5 surfaces.
- **2026-06-29** — **W4 re-scoped + scaffolded** (`docs/plans/20260629-spark-w4-shared-backend-factory.md`,
  registry row added). The "W4 = dialect lowerings" slot **dissolved**: an audit found all six
  spec-required Spark lowerings already implemented + unit-tested, and W3·P3 was a clean parity pass —
  so there is no dialect-lowering work to scaffold (live-execution coverage of the existing lowerings
  folds into W5). In its place, a **structural CLI↔UI parity fix** surfaced: backend selection is
  duplicated across `smelt-cli/src/backend_registry.rs` (DuckDB+Spark) and
  `smelt-ui/src/run_manager.rs`'s `UiBackendFactory` (**DuckDB-only**) — so a Spark project runs from
  `smelt run` but **cannot run from the UI at all**. W4 = extract one shared `smelt-backends` factory
  both consumers delegate to (P1 crate + CLI delegate → P2 UI delegate, gaining Spark → P3 dual-consumer
  guard test → P4 fail-loud on unknown backend `type:`, fixing the silent `_ => DuckDB` fallback).
  Spec diff landed in `architecture.md` §"Run pipeline parity rule (CLI ↔ UI)" (backend-selection
  contract + DO/DON'T bullets + the UI-DuckDB-only Mode-B incident). Dialect lowerings no longer a
  standalone wave; broad CLI mirror / independent Spark coverage renumbers to **W5**.
