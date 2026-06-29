# Plan: W7 — Delta-enabled CI gate + docs + Known-Divergence retractions

**Parent (master plan)**: `docs/plans/20260628-spark-parity.md` — the **W7** wave, the **final** one.
W1–W6 built the harness, exercised the Spark backend's exec/state ops, and made the capability matrix
honest + self-enforcing. W7 closes the initiative: provision the **Delta-enabled** Spark server the
parity baseline requires, get the full live-Spark suite green on it, gate it in CI, document the
backend, and retract the `multi_backend.md` Known-Divergences that are now genuinely true.

**Date**: 2026-06-30
**Spec**: `docs/specs/multi_backend.md` — the oracle. Specifically §Design *"Delta as the parity
baseline"* (Delta is required for MERGE, column mapping, and rich schema evolution — "Parquet format is
a documented, reduced-capability profile, not the parity target"), §"Cross-engine data exchange", and
the §Constraint *"Default `cargo test` is backend-agnostic … Spark coverage runs only in the gated job
that provides the server."*
**Spec diff**: partly landed alongside this plan (human gate, 2026-06-30) — in `multi_backend.md`:
retracted the now-true "Session init and Arrow loading not yet honored" divergence (honored since W2,
exercised live in W6); narrowed "Cross-engine type conformance … unvalidated" to "partly validated"
(decimal + `TIMESTAMP_NTZ` asserted in W6·P4; timezone-aware timestamp still open); narrowed the
"two provisional cells" divergence to the single `supports_nested_array_ddl`/Delta cell and recorded
that the test server lacks Delta. The remaining retractions ("parity not yet verified end to end"; the
timezone-aware timestamp clause; the Delta cell) are landed **by this wave's phases** as each becomes
green — see the per-phase notes.
**Tracking branch**: `worktree-spark`
**Docs**: spec (retractions, landed per-phase) + `docs-site/` backend pages + `CLAUDE.md` Commands
entry. This is the **docs-bearing** wave.

---

## Why a whole wave, not just "turn on CI"

W6·P2 surfaced the decisive infra gap: the test server (`apache/spark:latest`) **does not bundle Delta
Lake** — `CREATE TABLE … USING DELTA` fails `[DATA_SOURCE_NOT_FOUND]`. The spec names **Delta the parity
baseline** (MERGE, column mapping, schema evolution all require it), so a large slice of "parity" —
including W5's MERGE (P4) and schema-evolution (P5) tests, which ran **Spark-skipped** — has **never
executed against a Delta-capable live server**. W7 cannot honestly retract "parity is not yet verified"
until Delta is provisioned and the Delta-dependent suite is green. So W7 is: **provision Delta → get
green → gate in CI → document → retract**, in that order.

---

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then `docs/specs/multi_backend.md` §Design "Delta as the parity baseline" +
§"Cross-engine data exchange". Run the next `pending` phase in the Progress-tracking table (skip
`done`/`blocked` rows) using the per-phase routine below. After the last `pending` phase, flip this
sub-plan's row in the master registry (`docs/plans/20260628-spark-parity.md`) to `done`, update the
master Status, and commit together. This is the **final** wave: when its last phase lands and no other
registry row has pending work, the parity initiative is complete. Emit exactly one sentinel:
`<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>`, `<<SUBPLAN_ADVANCED>>`, or `<<MASTER_EXHAUSTED>>` (the latter
when W7 is fully `done` — it surfaces the completed initiative to a human).

**Server requirements.** P2/P3/P4 need a **Delta-enabled** Spark Connect server (P1 builds it) with
`SPARK_CONNECT_URL` exported. When Spark is unset or Delta is absent, P2/P3 cannot verify and **block
+ record** rather than asserting a guess. P1 (scripts) and P5 (docs) are doable without a server, but
P5's retraction of "parity not verified" must **not** land until P3 + P4 are green.

---

## Goal

A `scripts/spark-up.sh` that provisions Delta Lake; the full live-Spark parity suite green against it
(including the W5 Delta-path tests that never ran live, plus timezone-aware timestamp round-trip); a
**gated CI job** that stands up the Delta server, runs `cargo test` with `SPARK_CONNECT_URL` + the spark
feature, and tears it down; a `docs-site/` backend page documenting DuckDB + Spark targets (incl. that
the UI now runs Spark) and a `CLAUDE.md` Commands entry; and the `multi_backend.md` Known-Divergences
retracted to match reality.

---

## Per-phase routine

1. **Pre-flight.** `cargo build 2>&1 | tail -30` compiles; `cargo test --quiet 2>&1 | tail -40` green.
   Red on **unrelated** breakage → block.
2. **Red-green.** Write the failing test/check named in the phase first, confirm red, implement the
   minimal change, confirm green. Implementer pass, then reviewer pass (material findings only).
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings) **and**
   `cargo clippy --all-targets --features smelt-cli/spark`; `cargo test --quiet 2>&1 | tail -40` green;
   the parity gate `cargo test -p smelt-runtime --test execute_parity`; the example gate
   `cargo test -p smelt-cli --test example_diagnostics`. For server phases with Delta live, run the
   relevant `--features smelt-cli/spark` tests with `SPARK_CONNECT_URL` set and confirm green. For docs
   phases, build the docs site if a build step exists.
4. **Record + commit.** Set the table row to `done` + date; commit + push with the phase commit message.
   Emit `<<PHASE_COMPLETE>>` (or the roll-up sentinel on the last phase).

---

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue)

Set the row to `blocked` + one-line reason; append a dated entry to §"Blocked phases"; restore a clean
committed tree; commit + push; emit `<<PHASE_BLOCKED>>`. Conditions:

- Pre-flight red on unrelated breakage this phase didn't introduce.
- **P1**: no compatible Delta package for the image's Spark version can be resolved (record the Spark
  version + the Delta versions tried).
- **P2/P3**: Spark unset or Delta still absent → cannot verify → block "needs Delta-enabled server".
  A matrix edit to record observed DDL truth is a human-gated spec change — set the **code** value,
  record the needed matrix edit, and block for a human to land it.
- **P4**: the CI runner cannot run the Spark container (resource/permission) — record and leave the job
  defined but gated off.

---

## Progress tracking

| Phase | Title | Status | Commit | Date |
|-------|-------|--------|--------|------|
| P1 | Provision Delta Lake on the Spark Connect test server (`spark-up.sh`) | done | feat(spark-w7): P1 — Delta-enabled spark-up.sh + delta_smoke green | 2026-06-30 |
| P2 | Resolve the blocked `supports_nested_array_ddl`/Delta cell on the Delta server | done | feat(spark-w7): P2 — nested_array_ddl/Delta cell resolved (true); conformance complete | 2026-06-30 |
| P3 | Full live-Spark parity suite green on the Delta server (incl. W5 Delta path + timestamp-TZ) | done | feat(spark-w7): P3 — full live-Spark parity suite green on Delta server | 2026-06-30 |
| P4 | Gated CI job: Delta server up → `cargo test --features spark` → down | pending | | |
| P5 | Docs (`docs-site/` backend page + `CLAUDE.md`) + Known-Divergence retractions | pending | | |

---

### Phase P1: Provision Delta Lake on the Spark Connect test server

**Goal.** Make `scripts/spark-up.sh` stand up a Spark Connect server **with Delta Lake**, so
`CREATE TABLE … USING DELTA` / `MERGE` work. This unblocks the parity baseline (W6·P2's blocker).

**Critical files.**
- `scripts/spark-up.sh:36-44` — the `spark-submit` invocation. Add Delta: `--packages
  io.delta:delta-spark_2.13:<ver>` (pin `<ver>` to the image's Spark version — the W6 block suggested
  `4.0.0`; verify against `apache/spark:latest` = Spark 4.1.x and bump if the resolve fails) plus the
  two confs `--conf spark.sql.extensions=io.delta.sql.DeltaSparkSessionExtension` and `--conf
  spark.sql.catalog.spark_catalog=org.apache.spark.sql.delta.catalog.DeltaCatalog`. Mind that
  `--packages` needs network on first run (Ivy resolve) — note the warm-up in the script.
- `scripts/README-spark.md` — document the Delta packages/confs and the one-time Ivy download.

**TDD check to write first** (a live-gated smoke, e.g. extend `crates/smelt-backend-spark/tests/`):
- `delta_table_create_and_merge_smoke()` — against the live server, `CREATE TABLE … USING DELTA`,
  `MERGE INTO …`, and read back. **Red today** (`[DATA_SOURCE_NOT_FOUND] DELTA`); green once `spark-up.sh`
  provisions Delta. Skip + record when `SPARK_CONNECT_URL` unset.

**Verification (P1).** `scripts/spark-up.sh` brings up a Delta-capable server; the Delta smoke is green;
the README documents it.

---

### Phase P2: Resolve the blocked Delta capability cell

**Goal.** Close W6·P2's remaining blocker. With Delta live (P1), empirically determine
`supports_nested_array_ddl` on Spark **Delta**; set the constructor (`dialect.rs`) and the matrix
(`multi_backend.md`) to the observed truth; remove any conformance-suite exclusion for that cell; retract
the narrowed "one provisional cell" Known-Divergence.

**Critical files.**
- `crates/smelt-dialect/src/dialect.rs:154` (`spark_delta().supports_nested_array_ddl`, currently `false`).
- `docs/specs/multi_backend.md` matrix row (`supports_nested_array_ddl` Delta currently `✓`) + the
  provisional-cell Known-Divergence.
- `crates/smelt-backend-spark/tests/ddl_observed.rs` (the W6·P2 file) + the conformance suite
  `crates/smelt-dialect/tests/capability_conformance.rs`.

**TDD test to write first.**
- `spark_delta_nested_array_ddl_observed()` — create a Delta table with an array-of-struct column, run
  the nested-array ALTER, record success/failure; set the constructor flag to the observed boolean.
  Then the conformance suite (asserting this cell) must agree with the matrix. **The matrix edit is
  human-gated**: if observed ≠ current matrix `✓`, set code, record the needed matrix change in
  §"Blocked phases", and block. When Delta unavailable: block "needs Delta-enabled server".

**Verification (P2).** With Delta live: the cell reflects observed Delta behavior (code + matrix agree);
the conformance suite has **no** remaining provisional exclusions; the Known-Divergence is gone.

---

### Phase P3: Full live-Spark parity suite green on the Delta server

**Goal.** Run the complete `--features smelt-cli/spark` suite with `SPARK_CONNECT_URL` against the
Delta-enabled server and make it green — in particular the Delta-dependent W5 tests that previously ran
**Spark-skipped** (W5·P4 MERGE/cumulative, W5·P5 schema evolution) and the **timezone-aware timestamp**
cross-engine round-trip W6·P4 left open. Fix each gap red→green.

**Critical files.**
- `crates/smelt-cli/tests/` — the W5 dual-target tests (`merge_parity.rs`, `schema_evolution_parity.rs`,
  `incremental_parity.rs`, …) now actually exercise their Spark targets.
- `crates/smelt-dialect/src/type_conformance.rs` — add timezone-aware timestamp handling if the
  round-trip shows TZ drift (W6·P4 only covered `TIMESTAMP_NTZ`).
- Spark backend / `ddl_spark.rs` for any Delta MERGE / schema-evolution gap surfaced live.

**TDD test to write first.**
- `timestamp_tz_roundtrips_spark_to_duckdb()` — a Spark model produces a timezone-aware timestamp,
  materialized to Parquet, read by DuckDB; assert the same instant (no silent TZ shift). Plus: confirm
  the W5 Delta tests pass live (they assert parity but only ran DuckDb so far). **Red** wherever a
  Delta-path op or TZ semantics diverge; fix → green.

**Verification (P3).** With Delta live: `cargo test --features smelt-cli/spark` (with `SPARK_CONNECT_URL`)
is **green across the parity suite**, including MERGE, schema evolution, and timezone-aware timestamp.
This is the "parity is green" precondition for P4/P5. When Delta unavailable: block.

---

### Phase P4: Gated CI job

**Goal.** A CI job that provisions the Delta server, runs the parity suite against it, and tears it down
— so parity is continuously verified, gated (nightly / labeled) to avoid burdening every PR.

**Critical files.**
- `.github/workflows/compat.yml` — add a `spark-parity` job alongside the existing `spark-integration`
  / `type-property-spark` jobs (same `if: github.event_name == 'schedule' || contains(labels, 'run-docker-tests')`
  gate). Steps: checkout → Rust + DuckDB setup → `bash scripts/spark-up.sh` (now Delta-enabled) →
  export `SPARK_CONNECT_URL=sc://localhost:15002` → `cargo test --features smelt-cli/spark` (the parity
  tests) → `bash scripts/spark-down.sh` (always, even on failure). Reuse the cargo cache pattern already
  in the file. Mind the host pyspark client / PYTHONPATH the tests need (`scripts/spark-env.sh`).
- The existing `spark-integration` job runs only `smelt-parser-compat`; the new job is the **dual-target
  harness** parity suite — distinct, not a rename.

**TDD shape.** CI YAML isn't unit-testable; validate by (a) `actionlint` / YAML parse if available, and
(b) confirming the exact command sequence runs green locally with Delta up (it must, from P3). Commit
with a note that the job is gated and how to trigger it (`run-docker-tests` label).

**Verification (P4).** The job is defined, gated, and its command sequence matches the P3-green local
run; a labeled run (or the reviewer's manual trace) shows up → test → down.

---

### Phase P5: Docs + Known-Divergence retractions

**Goal.** Document the backend surface for users and retract every `multi_backend.md` Known-Divergence
that P1–P4 made true.

**Critical files.**
- `docs-site/docs/` — add/extend a backends/targets page (near `reference/smelt-yml.md` §Target shape):
  DuckDB + Spark targets, the Delta vs Parquet profile distinction, that the **UI now runs Spark** (W4),
  and the cross-engine Spark→DuckDB Parquet exchange at a user level. Add to the nav if the site uses one.
- `CLAUDE.md` "Commands" section — a "Spark parity tests (gated)" entry: `scripts/spark-up.sh` (Delta) →
  `source scripts/spark-env.sh` → `cargo test --features smelt-cli/spark` → `scripts/spark-down.sh`,
  and the `run-docker-tests` CI label.
- `docs/specs/multi_backend.md` Known-Divergences — retract, **only as each is green**:
  - "Parity is not yet verified end to end" → retract once P3 (suite green) + P4 (CI job) land. Rewrite
    to state parity is verified by the gated job against a Delta-enabled server.
  - The timezone-aware timestamp clause of "Cross-engine type conformance … partly validated" → retract
    once P3's TZ test is green (leaving the divergence gone entirely if decimal + NTZ + TZ all hold).
  - Bump `last_reviewed`.

**Verification (P5).** Docs build (if a build step exists); `CLAUDE.md` entry present; the
`multi_backend.md` Known-Divergences reflect reality (no stale "unverified" claims for things now gated
in CI). `/smelt:validate multi_backend` (if run) shows no drift.

**Close-out.** When P5 is committed: flip W7's row in `docs/plans/20260628-spark-parity.md` to
`done (<date>)`, update the master Status to record the **initiative complete**, commit together. No
further waves remain in the scaffolding queue — the loop surfaces the completed initiative to a human.

---

## Deferred (not in W7)

- Partition-pruned cross-engine reads (perf, not correctness) → deferred per `multi_backend.md`.
- Databricks as a distinct backend / real Spark materialized views → out of scope (no Databricks backend).
- The un-mirrored DuckDB integration areas logged in W5's "Coverage gaps deferred" (function expansion,
  cohort/selector e2e, full backfill on Spark) → a future coverage pass if desired, not required for the
  parity contract.

---

## Blocked phases

_(none yet)_

---

## Verification (wave-level, after P5)

- `cargo build` + `cargo test --quiet 2>&1 | tail -40` green; `cargo clippy --all-targets` and
  `--features smelt-cli/spark` zero warnings.
- `scripts/spark-up.sh` provisions Delta; `CREATE TABLE … USING DELTA` + `MERGE` work.
- With Delta live: `cargo test --features smelt-cli/spark` (with `SPARK_CONNECT_URL`) is green across the
  parity suite — MERGE, schema evolution, nested-array DDL cell, decimal + NTZ + timezone-aware timestamp.
- The capability-conformance suite asserts **every** matrix cell with no provisional exclusions.
- The gated `spark-parity` CI job is defined and its command sequence matches the green local run.
- `docs-site/` documents DuckDB + Spark targets (incl. UI-runs-Spark); `CLAUDE.md` has the Spark-tests
  entry; `multi_backend.md` Known-Divergences carry no stale "unverified" claims.
