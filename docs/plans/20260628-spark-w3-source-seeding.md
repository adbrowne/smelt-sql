# Plan: W3 — Source seeding in the dual-target smoke + re-smoke

**Parent (master plan)**: `docs/plans/20260628-spark-parity.md` — the **W3** wave. W3 makes the
dual-target smoke **seed source data identically into both backends** before running models, so the
pipeline gets past BL-3/BL-4 (`[TABLE_OR_VIEW_NOT_FOUND] analytics.sources_raw_sessions`), then
**re-smokes** so the real dialect/exec breaks — which the missing source table was hiding — finally
surface and scope W4.

**This is a test-harness wave, not a product feature.** Per the human-gate decision (2026-06-28),
smelt's lack of a pipeline source-materialization step is a known product gap to be decided
separately; W3 does **not** add one. It seeds sources at the *test* level so parity can be verified
on equal footing. The same model + the same source data must produce the same result on both
backends — that is what the smoke asserts.

**Date**: 2026-06-28
**Spec**: `docs/specs/multi_backend.md` — §Semantics "Parity contract" (same model + same data →
same result) and §Constraints ("Default `cargo test` is backend-agnostic"; Spark skips when
`SPARK_CONNECT_URL` unset) are the oracle. **No spec change** — W3 changes test scaffolding only.
**Spec diff**: none.
**Tracking branch**: `worktree-spark`
**Docs**: code/infra-only (test harness). No `docs-site/` change. The `CLAUDE.md` "Commands" entry
and backend pages land in the CI-gate wave.

---

## Background (why this wave exists)

BL-3 (W2·P3) is **not** a Spark bug. smelt has **no source-materialization step in the run pipeline
for any backend** — `smelt.sources.*` refs compile correctly to a table name
(`make_path_ref_resolver_with_ephemerals` → `<schema>.<segs_joined>`, e.g.
`analytics.sources_raw_sessions`) but nothing creates that table. The DuckDB examples "work" only
because `examples/multi_engine/run.sh` / test harnesses **manually seed** the source tables before
running models. Spark exposed the gap because nothing pre-seeded its catalog. So for the *parity
smoke* to test anything downstream, the harness must seed the source table into **each** target the
same way. (Full source materialization in the pipeline is a separate product decision, deferred.)

---

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then `docs/specs/multi_backend.md` §Semantics "Parity contract" + §Constraints —
that is the oracle. Run the next `pending` phase in the Progress-tracking table (skip
`done`/`blocked` rows) using the per-phase routine below. After the last `pending` phase, flip this
sub-plan's row in the master registry (`docs/plans/20260628-spark-parity.md`) to `done` and commit
together. Emit exactly one sentinel: `<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>`,
`<<SUBPLAN_ADVANCED>>`, or `<<MASTER_EXHAUSTED>>`.

**Spark must be live for P3 to be meaningful** (P1/P2's DuckDb path runs without Spark; the Spark
path skips when `SPARK_CONNECT_URL` unset). A human runs `scripts/spark-up.sh` once and exports
`SPARK_CONNECT_URL` (see the master's "Prerequisite"). P3 **blocks** rather than recording an empty
result if Spark is unprovisioned.

---

## Goal

A reusable harness helper that materializes a source table into either target's catalog from
identical data, wired into the `examples/multi_engine` smoke for **both** `{DuckDb, Spark}`, so the
models execute real SQL on both. Then a re-smoke whose recorded break list captures the dialect/exec
breaks that the missing source table was masking — the input a human uses to scaffold **W4 (dialect
lowerings)**.

---

## Per-phase routine

1. **Pre-flight.** `cargo build --features spark -p smelt-backend-spark 2>&1 | tail -30` compiles;
   `cargo test --quiet 2>&1 | tail -40` is green (Spark tests skip when `SPARK_CONNECT_URL` unset).
   If red on **unrelated** breakage, treat as a block.
2. **Red-green.** Write the failing test(s) named in the phase first, confirm red, implement the
   minimal change, confirm green. Implementer pass, then reviewer pass (material findings only).
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets --features smelt-cli/spark` (zero
   warnings); `cargo test --quiet 2>&1 | tail -40` green; the example gate
   `cargo test -p smelt-cli --test example_diagnostics`.
4. **Record + commit.** Set the table row to `done` + date; commit + push tests + impl + table with
   the phase commit message. Emit `<<PHASE_COMPLETE>>` (or the roll-up sentinel on the last phase).

---

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue)

Set the row to `blocked` + one-line reason; append a dated entry to §"Blocked phases"; restore a
clean committed tree; commit + push; emit `<<PHASE_BLOCKED>>`. Conditions:

- Pre-flight red on unrelated breakage this phase didn't introduce.
- **P3-specific:** `SPARK_CONNECT_URL` unset/unreachable, so no real re-smoke break list can be
  captured. Block with reason "Spark server not provisioned in loop env".
- A fix needs a redesign beyond this wave's surface (test harness only — **do not** add a
  pipeline/product source-load step; that is the deferred product decision).

---

## Progress tracking

| Phase | Title | Status | Commit | Date |
|-------|-------|--------|--------|------|
| P1 | Harness source-seed helper (materialize a source table into either target from identical data) | done | | 2026-06-29 |
| P2 | Wire source seeding into the `multi_engine` smoke for both targets (resolves BL-3/BL-4) | pending | | |
| P3 | Re-smoke; record the now-reachable dialect/exec breaks (→ W4) | pending | | |

---

### Phase P1: Harness source-seed helper

**Goal.** A reusable helper that, given a target backend and a source table name, materializes that
table from a fixed, deterministic dataset — identically for `DuckDb` and `Spark`. Both backends must
end up with the **same rows** so a downstream parity assertion is meaningful.

**Source data.** Produce one deterministic dataset for `raw.sessions`. Two acceptable routes
(implementer picks; prefer the first unless the schema is awkward):
- **Run `smelt-datagen`** (binary `CARGO_BIN_EXE_smelt-datagen`, config
  `examples/multi_engine/datagen.yaml`, output `data/sessions/**/*.parquet`) once into a temp dir,
  then load that Parquet into each backend. This reuses the example's real source schema.
- **Build a minimal in-memory Arrow `RecordBatch`** in Rust with exactly the columns
  `staging/stg_sessions.sql` reads (confirm the column set from the model SQL first). Smaller and
  fully deterministic; no subprocess.

**Per-backend materialization** (both must yield identical rows — Spark must **not** read a host
path, per the W2 BL-1 contract):
- **Spark:** read the dataset into an Arrow batch in Rust and call the (W2-fixed) `load_table`
  (Arrow IPC → `createDataFrame` → `saveAsTable`) to create `analytics.sources_raw_sessions`.
- **DuckDb:** either the DuckDB backend's Arrow load path, or
  `CREATE TABLE analytics.sources_raw_sessions AS SELECT * FROM read_parquet('<datagen dir>/**/*.parquet')`
  against the same DuckDB database file the smoke uses. If using the Arrow-batch route, load the
  identical batch both places.

**Critical files.**
- `crates/smelt-cli/tests/common/mod.rs` — add `pub fn seed_source_table(target: &TargetKind, project_dir, table_fqn: &str, /* data handle */)`
  that materializes `table_fqn` (e.g. `analytics.sources_raw_sessions`) into the target's catalog.
  Reuse `targets_yaml`/`stage_dual_workspace` conventions for the connection params (DuckDb db path;
  Spark `connect_url`+`schema`+`warehouse`). The source table name is the compiled form
  `<schema>.<source-segs-joined-by-_>` — confirm via `make_path_ref_resolver_with_ephemerals`
  (`crates/smelt-runtime/src/compile.rs:~750`) and the example's
  `examples/multi_engine/models/sources/raw/sessions.yml` (no `name:` override → default join).
- Spark ingest path (reuse, don't reimplement): `crates/smelt-backend-spark/src/lib.rs` `load_table`
  (~398–450) + `python/smelt/spark_adapter.py` `load_arrow_table` (~66–89).
- DuckDB ingest: `crates/smelt-backend-duckdb/src/lib.rs` (its execute/load path).
- `crates/smelt-datagen/` (binary) + `examples/multi_engine/datagen.yaml` if using the datagen route.

**TDD test to write first** (`crates/smelt-cli/tests/common` self-test or a new
`crates/smelt-cli/tests/source_seed.rs`, `mod common;`):
- For each `kind in targets_to_run()`: stage a workspace, `seed_source_table(...)`, then assert the
  table exists with the expected row count (query via the same target).
- Red: helper doesn't exist / table absent. Green: DuckDb always seeds + asserts; Spark seeds +
  asserts when `SPARK_CONNECT_URL` set, skips otherwise.

**Verification (P1).** Per-phase routine, plus: with `SPARK_CONNECT_URL` unset the self-test seeds
only DuckDb and passes; with it set, both targets seed and the row counts match.

---

### Phase P2: Wire source seeding into the smoke

**Goal.** The `examples/multi_engine` smoke seeds `analytics.sources_raw_sessions` into **each**
target via the P1 helper before `smelt run`, so `staging.stg_sessions` and
`intermediate.int_visitor_daily` find their inputs. BL-3 and BL-4 resolve; the models execute.

**Critical files.**
- `crates/smelt-cli/tests/spark_smoke.rs` `spark_smoke_multi_engine` (~line 67) — before the
  per-target `smelt run`, call `seed_source_table` for each target. The DuckDb baseline and the
  Spark run must be seeded with the **same** data so the parity comparison is valid.
- Confirm the exact source table FQN the compiled models reference (re-read the W2 break list:
  `analytics.sources_raw_sessions`; the cascade `analytics.staging_stg_sessions` is a *model* output,
  not a source — it materializes once `stg_sessions` runs, so seeding the source alone should clear
  both BL-3 and BL-4).

**TDD shape.** The smoke is the test. After P2, `stg_sessions` and `int_visitor_daily` should no
longer fail with `[TABLE_OR_VIEW_NOT_FOUND]`. If they now fail on a *dialect* construct instead,
that is success for P2 (BL-3/BL-4 cleared) and becomes P3's recorded break. Keep the collect-don't-
abort shape.

**Verification (P2).** Per-phase routine, plus: with Spark up, the smoke gets past BL-3/BL-4 (no
`TABLE_OR_VIEW_NOT_FOUND` for `analytics.sources_raw_sessions` / `analytics.staging_stg_sessions`).

---

### Phase P3: Re-smoke + record remaining breaks

**Goal.** With sources seeded on both backends, re-run the smoke so the models execute real SQL and
the actual dialect/exec breaks surface. Success is "the smoke ran the models on live Spark and the
remaining breaks (or a clean parity pass) are recorded", not "everything is green".

**Two possible outcomes — both are fine, record either:**
- The models hit a dialect lowering Spark rejects (QUALIFY, `DATE '…'`, `::`, `[…]`, trailing comma,
  …) → record each as **BL-5+** with the failing construct + Spark error + suspected flag/lowering.
  These scope **W4 (dialect lowerings)**.
- The `multi_engine` models are simple enough that Spark matches the DuckDb baseline → record a
  **parity pass for `examples/multi_engine`** and note that dialect-break discovery must come from a
  richer model set (the broad CLI mirror) instead — i.e. W4 is sourced from there, not here.

**Critical files.**
- `crates/smelt-cli/tests/spark_smoke.rs` — re-run; collect `(model, error)` and assert/report.
- Append to this file's **"## Recorded break list"**: mark BL-3/BL-4 **resolved (W3)**; add BL-5+ for
  each new break (or record the parity pass). Mirror a one-line summary into the master Status and
  ensure the master "## Wave scaffolding queue" W4 bullet references the new breaks.

**P3 block condition.** `SPARK_CONNECT_URL` unset/unreachable → **block** ("Spark server not
provisioned"); do not flip P3 to `done` without a real re-smoke.

**Close-out.** When P3 is recorded and committed: flip W3's row in
`docs/plans/20260628-spark-parity.md` to `done (<date>)`, update the master Status, commit together.
The loop emits `<<MASTER_EXHAUSTED>>`, surfacing to a human to scaffold **W4** from the breaks.

---

## Deferred (not in W3)

- A **product** source-materialization step in the run pipeline (load `smelt.sources.*` into any
  backend before model execution) — a separate human design decision, **not** this wave.
- Dialect lowerings the re-smoke surfaces → **W4**.
- Broad CLI mirror over `{DuckDb, Spark}` → **W5**.
- Capability conformance + cross-engine type validation → **W6**.
- Gated CI job + `CLAUDE.md`/`docs-site` updates → **W7**.

---

## Recorded break list

Carries forward; P2 resolves BL-3/BL-4, P3 appends what the deeper smoke surfaces.

- **BL-3 (source table not materialized).** _Resolved in W3·P2 — the smoke seeds
  `analytics.sources_raw_sessions` into each target via the P1 helper before `smelt run`._ Original
  symptom: `[TABLE_OR_VIEW_NOT_FOUND] analytics.sources_raw_sessions` — smelt has no pipeline
  source-load step; the parity smoke seeds at the test level instead.
- **BL-4 (cascade — upstream model output absent).** _Resolved in W3·P2 — once the source is seeded,
  `stg_sessions` materializes `analytics.staging_stg_sessions`, so `int_visitor_daily` finds it._

_(P3 appends newly-surfaced dialect/exec breaks below.)_

## Blocked phases

_(none yet)_

---

## Verification (wave-level, after P3)

- `cargo test --quiet 2>&1 | tail -40` green with `SPARK_CONNECT_URL` unset (Spark tests skip).
- With Spark up + `SPARK_CONNECT_URL` set: the P1 seed self-test passes on both targets with matching
  row counts; the smoke gets past BL-3/BL-4 and the re-smoke result (remaining breaks or parity pass)
  is recorded.
- `cargo clippy --all-targets --features smelt-cli/spark` — zero warnings.
