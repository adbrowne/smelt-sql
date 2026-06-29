# Plan: W5 — Broad CLI mirror / independent Spark coverage

**Parent (master plan)**: `docs/plans/20260628-spark-parity.md` — the **W5** wave. W1–W4 built the
dual-target harness, unblocked the smoke (session init + Arrow seed load), seeded sources into both
engines, and gave both CLI and UI a shared backend factory. What is still **unverified** is whether
the Spark backend's *execution and state* operations — seed `load_table`, the six dialect lowerings,
incremental DELETE+INSERT, MERGE, schema evolution, materializations — actually behave the same as
DuckDB on a **live** Spark server. Most of that Spark code exists but has only ever run against a
synthetic `spark_ctx()` or skipped for want of a server. W5 makes parity *true* by parametrizing the
high-value exec/state CLI integration tests over `{DuckDb, Spark}` and fixing each real gap a live
run surfaces.

**Date**: 2026-06-29
**Spec**: `docs/specs/multi_backend.md` — the oracle. Specifically §"Parity contract" (the same model
on any backend yields the same logical result; the **Required lowerings when the corresponding flag
is `false`** list, lines 75–88), the **Capability matrix** (lines 34–56), §"Session initialization",
§"Loading data into a backend", and §"Incremental & schema evolution per backend".
**Spec diff**: **none** — W5 is a *verification* wave. It changes no user-visible surface or
semantics; it makes the already-specified parity contract executable on a live backend and fixes
printer/backend/adapter bugs where reality diverges from the spec. (The "Parity is not yet verified"
and "Session init / Arrow loading not yet honored" Known-Divergences in `multi_backend.md` are
**retracted in W7**, once the gated CI job runs green — not here.)
**Tracking branch**: `worktree-spark`
**Docs**: code + tests only. No `docs-site/` change in this wave (the backend-support note and the
Known-Divergence retraction ride with **W7**). No spec edit.

---

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then `docs/specs/multi_backend.md` §"Parity contract" + the capability matrix — that
is the oracle. Run the next `pending` phase in the Progress-tracking table (skip `done`/`blocked`
rows) using the per-phase routine below. After the last `pending` phase, flip this sub-plan's row in
the master registry (`docs/plans/20260628-spark-parity.md`) to `done` and commit together. Emit
exactly one sentinel: `<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>`, `<<SUBPLAN_ADVANCED>>`, or
`<<MASTER_EXHAUSTED>>`.

**This wave only delivers value with a live Spark server.** Unlike W4 (a structural refactor proven
on the DuckDb path), the *point* of W5 is live-Spark execution. Each phase's Spark assertions **skip
green** when `SPARK_CONNECT_URL` is unset — but a phase that ran with Spark unset has asserted only
the DuckDb side and **must record "Spark coverage skipped — re-run with SPARK_CONNECT_URL" on its
row** rather than claiming Spark parity. A human runs the loop with `scripts/spark-up.sh` live +
`SPARK_CONNECT_URL` exported (see the master plan's Prerequisite) so these phases actually exercise
Spark.

---

## Goal

The high-value exec/state CLI integration tests run against **both** `{DuckDb, Spark}` via the W1
harness and pass on a live Spark Connect server, with every gap a live run surfaces fixed by a
dialect lowering, backend op, or adapter fix. Concretely: a CSV seed loads end-to-end through
`smelt seed`/`smelt build` on Spark; all six false-flag lowerings execute on a real server; and
incremental DELETE+INSERT, MERGE (Delta), schema evolution, and view/materialized-view
materialization all produce the same logical result on Spark as on DuckDB. **Not** a goal:
mechanically mirroring all ~130 DuckDB integration tests, or the flag-by-flag capability-conformance
suite (that is **W6**).

---

## Per-phase routine

1. **Pre-flight.** `cargo build 2>&1 | tail -30` compiles; `cargo test --quiet 2>&1 | tail -40` is
   green. If red on **unrelated** breakage, treat as a block.
2. **Red-green (dual-target).** Write the failing dual-target test(s) named in the phase first. With
   `SPARK_CONNECT_URL` live, confirm the Spark target is **red** (the gap) while DuckDb is green;
   implement the minimal printer/backend/adapter fix; confirm both green. Implementer pass, then
   reviewer pass (material findings only). If Spark is unset, the test asserts DuckDb only and skips
   Spark — record that on the row (see Execution prompt).
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings) **and**
   `cargo clippy --all-targets --features smelt-cli/spark`; `cargo test --quiet 2>&1 | tail -40`
   green; the standing parity gate `cargo test -p smelt-runtime --test execute_parity`; the example
   gate `cargo test -p smelt-cli --test example_diagnostics`. When Spark is live, also run the new
   dual-target test with `SPARK_CONNECT_URL` set and confirm the Spark target passes.
4. **Record + commit.** Set the table row to `done` + date (note "Spark skipped" if applicable);
   commit + push tests + impl + table with the phase commit message. Emit `<<PHASE_COMPLETE>>` (or
   the roll-up sentinel on the last phase).

---

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue)

Set the row to `blocked` + one-line reason; append a dated entry to §"Blocked phases"; restore a
clean committed tree; commit + push; emit `<<PHASE_BLOCKED>>`. Conditions:

- Pre-flight red on unrelated breakage this phase didn't introduce.
- A live-Spark gap whose fix needs more than a printer lowering / backend-op / adapter change — e.g.
  a Delta runtime not available on the test server, or a Spark behaviour that needs a new capability
  flag (that is a spec change → human gate, since flags live in `multi_backend.md`).
- The dual-target assertion needs a harness capability that does not exist yet and cannot be added
  within this phase without reshaping `common/mod.rs` broadly (record it; a human can split a
  harness phase out).

---

## Progress tracking

| Phase | Title | Status | Commit | Date |
|-------|-------|--------|--------|------|
| P1 | Seed / `load_table` end-to-end parity on live Spark (+ reusable result-parity helper) | done (2026-06-29) — Spark skipped (SPARK_CONNECT_URL unset); re-run with server live | feat(spark-w5): P1 — seed load_table parity + fetch_rows/assert_table_parity helpers | 2026-06-29 |
| P2 | The six required dialect lowerings executed on live Spark | done (2026-06-29) — Spark skipped (SPARK_CONNECT_URL unset); re-run with server live. **ARRAY literal lowering** (`supports_array_literal`) not reachable via user SQL (smelt treats `ARRAY[...]` as a meta-language list; triggers `MetaListInScalarPosition`); covered by printer unit tests only — logged in §"Coverage gaps deferred". | feat(spark-w5): P2 — five dialect lowerings dual-target test (QUALIFY/DATE/cast/comma/CREATE OR REPLACE) | 2026-06-29 |
| P3 | Incremental DELETE+INSERT parity on live Spark | done (2026-06-29) — Spark skipped (SPARK_CONNECT_URL unset); re-run with server live | feat(spark-w5): P3 — incremental DELETE+INSERT idempotency dual-target test | 2026-06-29 |
| P4 | MERGE / cumulative parity on live Spark (Delta) | done (2026-06-29) — Spark skipped (SPARK_CONNECT_URL unset); re-run with server live | feat(spark-w5): P4 — MERGE cumulative upsert dual-target test | 2026-06-29 |
| P5 | Schema-evolution parity on live Spark | pending | | |
| P6 | Materialization parity (view / materialized-view fallback) + coverage-gap log | pending | | |

---

### Phase P1: Seed / `load_table` end-to-end parity on live Spark

**Goal.** Prove a CSV seed loads into a live Spark backend **through the real CLI path**
(`smelt seed` / `smelt build`), not just the W3 helper that calls `seed_source_table` directly. This
exercises `Backend::load_table` on Spark end-to-end (the BL-1 path, "fixed" in W2·P2 by sending Arrow
IPC bytes via Connect `createDataFrame`). Also establish the **result-parity helper** every later
phase reuses.

**Critical files.**
- `crates/smelt-cli/tests/common/mod.rs` — add a `fetch_rows(target_name, schema, table) -> Vec<Row>`
  (query the materialized table on a given target and return normalized rows) and an
  `assert_table_parity(...)` over `targets_to_run()`, **if not already present**. The DuckDb read can
  reuse the existing backend; the Spark read goes through the adapter's `execute_sql`. This helper is
  P1's reusable deliverable.
- `crates/smelt-cli/tests/cli_unit/seed_loading.rs` — the existing DuckDb `seeds_load_via_load_table()`
  is the model to mirror. The new test stages a workspace with a CSV seed (e.g. the
  `examples/timeseries/seeds/raw/users.csv` shape) and runs the **CLI** seed path per target.
- Spark side if a gap surfaces: `crates/smelt-backend-spark/src/lib.rs:398-466` (`load_table`),
  `python/smelt/spark_adapter.py:66-89` (`load_arrow_table`).

**TDD test to write first** (`crates/smelt-cli/tests/` — a new `seed_parity.rs` or extend
`source_seed.rs`):
- `seed_loads_into_both_backends()` — over `targets_to_run()`: stage a CSV seed + sidecar, run
  `smelt seed` (or `smelt build`) against the target, then `assert_table_parity` the seeded table has
  the expected rows/types. **Red on Spark** if the real CLI seed path has a gap; green after the fix.
  DuckDb green throughout; Spark skips green when `SPARK_CONNECT_URL` unset.

**Verification (P1).** Per-phase routine; with Spark live the new test's Spark target passes;
`seed_loading.rs` (DuckDb) still green.

---

### Phase P2: The six required dialect lowerings executed on live Spark

**Goal.** Execute each false-flag lowering on a **real** Spark server. Today they are unit-tested only
with a synthetic `spark_ctx()` (`crates/smelt-dialect/src/printer.rs`); W3·P3 was a clean parity pass
but `examples/multi_engine` is too simple to use any of them. Oracle: `multi_backend.md`
§"Parity contract" Required-lowerings list (lines 75–88) + the capability matrix.

**Critical files.**
- `crates/smelt-dialect/src/printer.rs` — the lowering implementations (QUALIFY-wrap, `to_date(...)`,
  `CAST(x AS T)`, `ARRAY(...)`, trailing-comma suppression, `DROP TABLE IF EXISTS` + `CREATE TABLE`).
- `crates/smelt-dialect/src/dialect.rs` — `BackendCapabilities` / `SqlDialect`.

**TDD test to write first** (`crates/smelt-cli/tests/lowering_parity.rs`):
- One dual-target model (or one test per construct) exercising all six triggers: a `QUALIFY` window
  filter, a `DATE 'YYYY-MM-DD'` literal, an `x::T` cast, a `[a, b]` array literal, a trailing comma in
  a select/group list, and a `CREATE OR REPLACE TABLE` materialization. Run on both targets; assert
  the model executes and `assert_table_parity` holds. **Red on Spark** for any construct the printer
  emits in a form a real server rejects; fix the printer → green. (Each lowering is unit-tested, so a
  live red here means the unit test's synthetic context diverged from the real server — capture it as
  an explicit case before fixing, per the project's property-test-failure rule.)

**Verification (P2).** Per-phase routine; with Spark live all six constructs run on Spark and match
DuckDb; the existing printer unit tests stay green.

---

### Phase P3: Incremental DELETE+INSERT parity on live Spark

**Goal.** Exercise Spark's incremental DELETE+INSERT path (`delete_partitions_range` + `insert_into`,
`crates/smelt-backend-spark/src/lib.rs:352-372` + `src/sql.rs`) on a live server — implemented but
never run live. Mirror the core incremental idempotency behaviour over both backends.

**Critical files.**
- `crates/smelt-cli/tests/cli_unit/incremental_test.rs`, `crates/smelt-cli/tests/incremental/` (the
  DuckDb idempotency / run-window tests to mirror — pick the minimal idempotency case).
- Spark: `crates/smelt-backend-spark/src/lib.rs:352-372`, `src/sql.rs` (`delete_partitions_range`,
  `insert_into`). Row-level DELETE on Spark requires a Delta table — the Spark target's default.

**TDD test to write first** (`crates/smelt-cli/tests/incremental_parity.rs`):
- `incremental_delete_insert_is_idempotent_on_both()` — run an incremental model for a window, re-run
  the same window, and `assert_table_parity` that the target table holds exactly the expected rows
  (no duplication) on **each** backend. **Red on Spark** if the DELETE window or INSERT diverges from
  DuckDb; fix → green. Skip-green when Spark unset.

**Verification (P3).** Per-phase routine; with Spark live the re-run is idempotent on Spark and rows
match DuckDb; DuckDb incremental tests still green.

---

### Phase P4: MERGE / cumulative parity on live Spark (Delta)

**Goal.** Exercise Spark `MERGE INTO` (`merge_into`, `lib.rs:374-384` + `sql.rs:80-92`) — requires
Delta (`supports_merge = ✓` only for Spark-Delta). Mirror the cumulative/MERGE end-to-end test.

**Critical files.**
- `crates/smelt-cli/tests/backbuild_cumulative_e2e.rs`, `cumulative_classifier_gate.rs` (DuckDb models
  to mirror).
- Spark: `crates/smelt-backend-spark/src/lib.rs:374-384`, `src/sql.rs:80-92` (`merge_into`).

**TDD test to write first** (`crates/smelt-cli/tests/merge_parity.rs`):
- `cumulative_merge_matches_across_backends()` — run a cumulative model that MERGEs new rows into an
  existing target across two batches; `assert_table_parity` the final state matches on both backends.
  **Red on Spark** if MERGE is rejected (e.g. non-Delta table, ON/UPDATE-SET shape) or yields a
  different final state; fix → green. If the test server lacks a Delta runtime, that is a **block**
  (record it for the human — provisioning, not a code fix).

**Verification (P4).** Per-phase routine; with Spark+Delta live the merged state matches DuckDb.

---

### Phase P5: Schema-evolution parity on live Spark

**Goal.** Exercise add-column / widening migration on Spark (`supports_merge_schema_write = ✓`,
`supports_column_mapping = ✓` for Delta; `crates/smelt-state/src/ddl_spark.rs`). Mirror the DuckDb
schema-evolution test.

**Critical files.**
- `crates/smelt-cli/tests/incremental/schema_evolution.rs`, `schema_roundtrip.rs` (DuckDb cases).
- Spark: `crates/smelt-state/src/ddl_spark.rs` (Spark DDL for migrations).

**TDD test to write first** (`crates/smelt-cli/tests/schema_evolution_parity.rs`):
- `add_column_migration_matches_across_backends()` — materialize a model, evolve it (add a nullable
  column), re-run, and `assert_table_parity` the migrated table preserves prior rows and exposes the
  new column on **each** backend. **Red on Spark** if the migration emits invalid Spark DDL or loses
  data; fix `ddl_spark.rs` → green. Skip-green when Spark unset.

**Verification (P5).** Per-phase routine; with Spark live the add-column migration succeeds on Spark
and matches DuckDb.

---

### Phase P6: Materialization parity (view / materialized-view fallback) + coverage-gap log

**Goal.** Cover `table` / `view` / `materialized_view` materialization across both backends. Spark
materialized-view falls back to a table with a warning (`supports_materialized_views = ✓` for Spark
but the adapter currently routes MV → `create_table_as`; `lib.rs:243-253` view, `lib.rs:309-317` MV
default) — assert the chosen materialization behaves and is consistent with the capability flag. Then
**log the remaining un-mirrored DuckDb integration areas** so the cap is explicit, not silent.

**Critical files.**
- e2e materialization coverage under `crates/smelt-cli/tests/e2e/` (pick a view + a table model).
- Spark: `crates/smelt-backend-spark/src/lib.rs:226-253` (`create_table_as` / `create_view_as`),
  `lib.rs:309-317` (MV default).

**TDD test to write first** (`crates/smelt-cli/tests/materialization_parity.rs`):
- `view_and_table_materialize_consistently_on_both()` — a `view` model and a `table` model; assert
  both materialize and `assert_table_parity` the queryable result matches per backend. Note the MV
  fallback behaviour explicitly (a `materialized_view` model on Spark should produce a queryable
  relation; if the flag claims MV support but the adapter falls back to a table, record the
  discrepancy — fixing the flag-vs-behaviour mismatch is **W6** conformance, not here, unless the
  result is simply wrong).

**Coverage-gap log (no silent cap).** Append to this plan's §"Coverage gaps deferred" a short list of
DuckDb integration areas W5 did **not** mirror to Spark (e.g. selectors, show-plan, function-schema
e2e) so W6/W7 or a follow-up can pick them up. Do not present W5 as "all DuckDb tests mirrored".

**Verification (P6).** Per-phase routine; with Spark live view/table parity holds; the coverage-gap
log is written.

**Close-out.** When P6 is committed: flip W5's row in `docs/plans/20260628-spark-parity.md` to
`done (<date>)`, update the master Status, commit together. The loop emits `<<MASTER_EXHAUSTED>>`,
surfacing to a human to scaffold **W6 (capability conformance + cross-engine)**.

---

## Deferred (not in W5)

- Flag-by-flag `BackendCapabilities` conformance suite + Spark→DuckDB Parquet type conformance
  (decimal precision, timestamp TZ) → **W6**.
- Gated CI job (`spark-up.sh` → `cargo test --features spark` → `spark-down.sh`), `CLAUDE.md`
  command entry, `docs-site/` backend pages + backend-support note, and retracting the
  `multi_backend.md` "parity not yet verified" / "session-init & Arrow loading not yet honored"
  Known-Divergences → **W7**.
- Partition-pruned cross-engine reads (perf, not correctness) → deferred per `multi_backend.md`.
- Real source-materialization in the run pipeline — **not a feature** (sources are external;
  `docs/specs/sources.md`). See the master plan's struck "source materialization" note.

---

## Coverage gaps deferred

**P2 finding — `supports_array_literal` lowering not reachable via user SQL:**
In smelt, `ARRAY[a, b]` is always parsed as a smelt meta-language list literal, not as a SQL
array literal.  A bare `ARRAY[...]` in a scalar SELECT position triggers `MetaListInScalarPosition`
(design decision in `docs/specs/meta_language.md`).  The `print_array_rewrite` lowering (`ARRAY[...]
→ ARRAY(...)`) fires only for compiler-generated SQL (e.g. inside expanded smelt function bodies),
never for user-written model SQL.  Consequence: this lowering cannot be covered by a CLI integration
test against a live Spark server — it is tested only by the printer unit tests in
`crates/smelt-dialect/src/printer.rs` (lines ~2081–2098).  The coverage gap is limited: any smelt
function that generates array-valued SQL already exercises the lowering via the printer unit tests;
a live-Spark execution test would only add value if a smelt function produces array-valued SQL that
is passed to Spark, which is not yet a scenario in the codebase.

_(P6 appends additional un-mirrored DuckDb integration areas here.)_

---

## Blocked phases

_(none yet)_

---

## Verification (wave-level, after P6)

- `cargo build` + `cargo test --quiet 2>&1 | tail -40` green; `cargo clippy --all-targets` and
  `--features smelt-cli/spark` zero warnings.
- `cargo test -p smelt-runtime --test execute_parity` green; `cargo test -p smelt-cli --test example_diagnostics` green.
- With `SPARK_CONNECT_URL` live: the seed, lowerings, incremental DELETE+INSERT, MERGE,
  schema-evolution, and materialization dual-target tests all pass on Spark and match DuckDb.
- The reusable `assert_table_parity` helper exists in `common/mod.rs` and is used by every phase.
- §"Coverage gaps deferred" records what W5 did not mirror — no silent cap.
