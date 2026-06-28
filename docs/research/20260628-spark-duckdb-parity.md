# Spark → DuckDB parity (design / brainstorm)

**Date**: 2026-06-28
**Status**: design approved (interactive), feeds `docs/specs/multi_backend.md` + the
`docs/plans/20260628-spark-parity.md` autonomy-loop master.

## Problem

smelt already ships a substantial Spark backend (`crates/smelt-backend-spark`, a PyO3 →
`python/smelt/spark_adapter.py` bridge, a `SparkSQL` dialect with a `BackendCapabilities`
matrix, a Docker `spark-sql` type oracle, cross-engine Parquet exchange, and an
`examples/multi_engine/` pipeline). The gap is **not** that Spark is unimplemented — it is
that almost none of it is **verified end to end**: the Spark integration tests are gated
behind `SPARK_CONNECT_URL`, no Spark runs in CI or on the dev box, and DuckDB by contrast is
exercised by ~50 CLI integration tests + example builds + property tests. "Spark matches
DuckDB" therefore means: **prove behavioural parity with a dual-target test matrix, and fix
each real gap the failures expose.**

## Goal

Verification-first, empirical. Stand up a real local Spark, mirror the bulk of DuckDB's test
surface against it, and let actual failures drive the dialect/exec fixes. End state: an
identical smelt model/project behaves the same on Spark as on DuckDB, proven by a dual-target
test matrix with a (gated) CI job, and a normative `multi_backend.md` spec that keeps the
capability matrix honest.

## Decisions (resolved interactively)

| # | Decision | Choice |
|---|----------|--------|
| 1 | Parity meaning | **Both, verification first** — run Spark, see what breaks, then fix the real gaps the failures surface (empirical, not spec-guessed). |
| 2 | Spark runtime | **Docker Spark Connect** primary (matches existing `SPARK_CONNECT_URL` gating + the oracle's container); host-local PySpark documented as fallback. The Connect *client* is pure-gRPC Python — no host JVM needed; only the in-container server needs a JDK, so the host Java 21 is irrelevant. |
| 3 | Test coverage | **Broad parametrized mirror** — parametrize the bulk of the ~50 DuckDB CLI integration tests to also run on Spark via a reusable dual-target harness, fixing each gap found. |
| 4 | Table format | **Delta primary** — Delta is the Spark default capability and is what gives MERGE + column-mapping + schema-evolution parity with DuckDB. Parquet format is a lighter secondary matrix. |
| 5 | Delivery | **Autonomy loop** — wired as a dedicated master + just-in-time sub-plan waves, not hand-implemented. |

## Architecture / components

### 1. Local Spark runtime (persistent service — set up once, outside the loop)
A scripted, reproducible Spark Connect server in Docker. **It is a long-lived service**, not
started per test: the autonomy loop's iterations are stateless `claude --print` processes, so
Spark must already be running and reachable at a stable `SPARK_CONNECT_URL` that every
iteration's gated tests connect to.
- `scripts/spark-up.sh` / `scripts/spark-down.sh` — start/stop the Connect server (Delta
  enabled), publish `:15002`, and **mount a host-shared warehouse dir** so host-side DuckDB
  can read Spark-produced Parquet for the cross-engine path.
- A pinned Python venv with `pyspark[connect]` (+ `delta-spark`) that the PyO3 adapter
  imports.
- The same container hosts the existing `spark-sql` type oracle — one Spark, both consumers.

### 2. Dual-target test harness
A reusable helper in `crates/smelt-cli/tests` exposing `TargetKind { DuckDb, Spark }`, a
per-kind `smelt.yml` target-block builder, and parametrization that runs an existing CLI
integration test against **both** backends. **Spark cases auto-skip when `SPARK_CONNECT_URL`
is unset**, so a plain `cargo test` stays green; the gated Spark job sets the env var. The
first implementation step is a discovery probe of how current CLI tests build their temp
projects — that is the injection point.

### 3. Gap-closing (empirically driven, dialect lowering)
Each real failure → a red dual-target test → a fix in the **dialect printer / planner
lowering** so identical logical SQL emits Spark-valid physical SQL when a
`BackendCapabilities` flag is false: QUALIFY → subquery row-filter, `DATE '…'` → `to_date`,
`a::T` → `CAST(a AS T)`, `[a,b]` → `ARRAY(a,b)`, trailing-comma strip, CREATE-OR-REPLACE-TABLE
emulation, etc. The capability matrix already exists; the work is making the printer **honor**
it rather than emitting invalid SQL.

### 4. Spec, CI gate, docs
- **`docs/specs/multi_backend.md`** — `architecture.md` already names this as a deferred spec.
  It is the normative oracle: backend parity contract + capability matrix + cross-engine
  rules.
- A gated CI job: `spark-up.sh` → `cargo test --features spark` with `SPARK_CONNECT_URL` set →
  `spark-down.sh`. Documented in `CLAUDE.md` "Commands".
- `docs-site/` backend pages updated.

## Delivery via the autonomy loop

- This worktree (`/.claude/worktrees/spark`, branch `worktree-spark`) is the checkout the loop
  drives — running from here pushes `worktree-spark` in isolation.
- The prior master (`docs/plans/20260613-spec-impl.md`, spec-remediation) is fully `done`, so
  repointing `.claude/active-plan` to a dedicated Spark-parity master clobbers nothing.
- **Just-in-time waves** (the loop's human-gated model): only **W1** (runtime + harness +
  smoke + break-list capture) is scaffolded now. W1 produces the empirical break list; a human
  then scaffolds W2+ ("fix the gaps") from those concrete failures and adds their registry
  rows. The loop never scaffolds a wave or edits a spec autonomously.

## Why not the alternatives

- **Host-local PySpark**: couples to host Java/Spark versions and pollutes the host Python
  env; less CI-like. Kept as a documented fallback only.
- **Capability-conformance-only** (one test per flag, no CLI mirror): cleaner but misses the
  emergent integration failures (state, incremental, cross-engine) that only show up running
  real projects. The broad mirror is the empirical net; a conformance suite can be added later
  as a W-wave if the mirror leaves capability gaps.
- **Guess-and-fix the dialect gaps up front** (feature-parity first): risks building lowerings
  for cases that never break and missing ones that do. Verification-first inverts this.

## Open questions (for W2+ scaffolding, not blocking W1)

- Delta + Spark Connect config specifics (extension packages, catalog) — settle empirically in
  W1 stand-up.
- Whether the cross-engine warehouse path needs remote storage (S3/GCS) — currently local-FS
  only; out of scope unless a mirrored test demands it.
- Partition-pruned cross-engine reads (today: full Parquet glob) — perf, not correctness;
  deferred.

## References
- Spec: `docs/specs/multi_backend.md` (authored alongside this work)
- Master plan: `docs/plans/20260628-spark-parity.md`
- W1 sub-plan: `docs/plans/20260628-spark-w1-runtime-harness.md`
- Existing: `docs/plans/20260328-multi-engine-example.md`, `docs/specs/architecture.md`
  §"Backend trait surface", `crates/smelt-dialect/src/dialect.rs`,
  `crates/smelt-backend-spark/`, `crates/smelt-db/tests/prop_helpers/spark_oracle.rs`.
