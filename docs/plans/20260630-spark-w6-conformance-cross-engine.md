# Plan: W6 — Capability conformance + cross-engine type validation

**Parent (master plan)**: `docs/plans/20260628-spark-parity.md` — the **W6** wave. W1–W5 built the
dual-target harness and exercised the Spark backend's exec/state operations. W6 makes the **capability
matrix** honest and self-enforcing, and validates the **cross-engine Spark→DuckDB Parquet boundary**
that no test has yet exercised end-to-end.

**Date**: 2026-06-30
**Spec**: `docs/specs/multi_backend.md` — the oracle. Specifically the **Capability matrix** (§Surface)
and the §Constraints clause *"The capability matrix table in §Surface and the `BackendCapabilities`
constructors agree. A conformance test asserts each flag of `::duckdb()`, `::spark_delta()`,
`::spark_parquet()` equals the table. Changing one without the other is a spec-vs-code drift the
conformance test must fail on."*, plus §"Cross-engine data exchange" and the §Constraint *"A `false`
capability never reaches the user as a diagnostic. Every `false` flag has a corresponding printer
lowering."*
**Spec diff**: landed alongside this plan (human gate, 2026-06-30) — in `multi_backend.md`: (a) flipped
`supports_materialized_views` to `✗ (table fallback)` for both Spark profiles (OSS Spark has no native
MV; the backend already falls back to a table); (b) added the `supports_pipe_syntax` row (all `✗`,
reconciling the struct field that was missing from the matrix); (c) generalized the MV lowering note;
(d) added a Known-Divergence recording that two cells (`supports_struct_field_ddl` on Parquet,
`supports_nested_array_ddl` on Delta) are **provisional pending live-Spark verification** in this wave.
The remaining `multi_backend.md` Known-Divergences (cross-engine type conformance unvalidated; parity
not yet verified) are **retracted in W7**, once the gated CI job runs green — not here.
**Tracking branch**: `worktree-spark`
**Docs**: spec (the matrix edits above, already landed) + code + tests. No `docs-site/` change in this
wave (rides with **W7**).

---

## Design decisions (recorded — human delegated/decided at the gate)

- **Spark materialized views → table fallback (flag `false`).** OSS Spark SQL has no native
  materialized view (it is a Databricks-only capability); the Spark backend already inherits the
  default `Backend::create_materialized_view_as` that creates a plain table + `warn!`. W6 makes the
  flag honest: `supports_materialized_views = false` in `spark_delta()`/`spark_parquet()`, so a
  `materialized_view` model routes through the same capability-gated table fallback as DuckDB
  (`smelt-backend/src/lib.rs:134` else-branch), not the misleading default-trait warn path. *Rejected:
  implementing a real Spark MV — only meaningful on Databricks (not a distinct backend yet, per
  `multi_backend.md` Known-Divergences) and unverifiable against the OSS Connect test server.*
- **Two DDL cells resolved empirically, not by assertion.** `supports_struct_field_ddl` (Parquet)
  and `supports_nested_array_ddl` (Delta) are matrix-vs-code disagreements about real Spark/Delta DDL
  behavior. W6 runs the actual ALTER against a live server and sets **both** the constructor and the
  matrix to the observed result, rather than guessing which side was right. Until verified they sit in
  the spec's Known-Divergences and are excluded from the conformance assertion (logged, not silent).
- **Conformance suite is pure Rust (no Spark server).** Asserting constructor flags against the matrix
  needs no live backend, so the standing drift gate (P1) runs in the default `cargo test` and is fully
  green without Spark. Only the empirical DDL check (P2) and the cross-engine phases (P3/P4) need a
  live server.

---

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then `docs/specs/multi_backend.md` Capability matrix + §"Cross-engine data exchange" —
that is the oracle. Run the next `pending` phase in the Progress-tracking table (skip `done`/`blocked`
rows) using the per-phase routine below. After the last `pending` phase, flip this sub-plan's row in
the master registry (`docs/plans/20260628-spark-parity.md`) to `done` and commit together. Emit exactly
one sentinel: `<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>`, `<<SUBPLAN_ADVANCED>>`, or
`<<MASTER_EXHAUSTED>>`.

P1 needs no Spark server. **P2/P3/P4 only deliver value with a live Spark server** (`SPARK_CONNECT_URL`
exported, `scripts/spark-up.sh` up — see the master plan's Prerequisite); when Spark is unset they
cannot determine the empirical truth (P2) or exercise the Parquet boundary (P3/P4), so they **record
the gap and block** (or skip + record) rather than asserting a guessed result.

---

## Goal

A standing capability-conformance suite asserts every `BackendCapabilities` constructor flag equals the
`multi_backend.md` matrix, so spec-vs-code drift fails CI. The two provisional DDL cells are resolved
to live-Spark truth. The cross-engine Spark→DuckDB `read_parquet` substitution works end-to-end through
the run pipeline (not just in unit tests), and decimal-precision + timestamp-timezone values round-trip
correctly across the Parquet boundary. **Not** a goal: the gated CI job, `docs-site` pages, or
retracting the remaining Known-Divergences (all **W7**).

---

## Per-phase routine

1. **Pre-flight.** `cargo build 2>&1 | tail -30` compiles; `cargo test --quiet 2>&1 | tail -40` green.
   Red on **unrelated** breakage → block.
2. **Red-green.** Write the failing test(s) named in the phase first, confirm red, implement the
   minimal change, confirm green. Implementer pass, then reviewer pass (material findings only).
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets` (zero warnings) **and**
   `cargo clippy --all-targets --features smelt-cli/spark`; `cargo test --quiet 2>&1 | tail -40` green;
   the parity gate `cargo test -p smelt-runtime --test execute_parity`; the example gate
   `cargo test -p smelt-cli --test example_diagnostics`. For P2/P3/P4 with Spark live, run the new
   test with `SPARK_CONNECT_URL` set and confirm the Spark path passes.
4. **Record + commit.** Set the table row to `done` + date; commit + push tests + impl + table with
   the phase commit message. Emit `<<PHASE_COMPLETE>>` (or the roll-up sentinel on the last phase).

---

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue)

Set the row to `blocked` + one-line reason; append a dated entry to §"Blocked phases"; restore a clean
committed tree; commit + push; emit `<<PHASE_BLOCKED>>`. Conditions:

- Pre-flight red on unrelated breakage this phase didn't introduce.
- **P2 needs a spec-matrix edit** to record the observed DDL truth — that is a human-gated spec change
  (flags live in `multi_backend.md`). Run the live DDL, set the **code** constructor to the observed
  value, record the observed truth + the matrix edit needed in §"Blocked phases", and block for a human
  to land the matrix change. (When Spark is unset, P2 cannot observe anything → block "undetermined,
  re-run with server".)
- **P3 cross-engine wiring** needs a `smelt-runtime`/compiler API change beyond connecting the existing
  `set_cross_engine_refs` machinery (i.e. the reference-resolution contract itself must change) — record
  for human review rather than reshaping the pipeline autonomously.
- A live-Spark gap whose fix needs a new capability flag or a Delta runtime the test server lacks.

---

## Progress tracking

| Phase | Title | Status | Commit | Date |
|-------|-------|--------|--------|------|
| P1 | Capability-conformance suite (constructors == matrix) + MV flag → false | done | feat(spark-w6): P1 — capability conformance suite + MV flag false | 2026-06-30 |
| P2 | Resolve the two provisional DDL cells empirically on live Spark | blocked | feat(spark-w6): P2 — struct-field DDL false on Parquet (empirical); Delta blocked (no Delta Lake) | 2026-06-30 |
| P3 | Wire the cross-engine Spark→DuckDB `read_parquet` substitution end-to-end | pending | | |
| P4 | Cross-engine Parquet type conformance (decimal precision + timestamp TZ) | pending | | |

---

### Phase P1: Capability-conformance suite + MV flag honesty

**Goal.** A standing test asserting **each** flag of `BackendCapabilities::duckdb()`, `::spark_delta()`,
`::spark_parquet()` equals the `multi_backend.md` matrix — the drift gate the spec §Constraint requires.
Plus the code half of the MV decision: flip `supports_materialized_views` to `false` for both Spark
constructors so they match the (already-edited) matrix.

**Critical files.**
- New `crates/smelt-dialect/tests/capability_conformance.rs` — encode the matrix (§Surface,
  `docs/specs/multi_backend.md` lines ~36–57) as the oracle: a table of `(flag_name, duckdb,
  spark_delta, spark_parquet)` expected booleans, asserted field-by-field against the three
  constructors. The conformance suite is the **executable** form of the matrix; when a flag changes,
  this test and the spec table change in the same commit.
- `crates/smelt-dialect/src/dialect.rs:150,177` — set `supports_materialized_views: false` in
  `spark_delta()` and `spark_parquet()` (currently `true`). DuckDB is already `false` (`dialect.rs:118`).
- **Exclude (logged, not silent)** the two provisional cells from the assertion this phase:
  `supports_struct_field_ddl`/Parquet and `supports_nested_array_ddl`/Delta. Mark them with an explicit
  `// PENDING live-Spark verification — W6 P2` and assert them in P2. Every other cell is asserted now.

**TDD test to write first.**
- `every_flag_matches_matrix()` — drives the `(flag, expected×3)` table against the three constructors.
  **Red** on the MV cells (code `true` vs matrix `false`) until the `dialect.rs` flip; green after.
- A guard that the expected-table is exhaustive over the struct's fields **minus** the two pending
  ones (so a newly-added flag forces a matrix + test update, not a silent omission).

**Verification (P1).** Per-phase routine (no Spark needed); `every_flag_matches_matrix()` green; the
snapshot tests in `crates/smelt-dialect/tests/snapshots.rs` still pass; a deliberate one-flag edit to a
constructor makes the conformance test fail (drift-gate self-check, can be a `#[test]` comment or a
manual check noted in the commit).

---

### Phase P2: Resolve the two provisional DDL cells empirically on live Spark

**Goal.** Determine the real behavior of `supports_struct_field_ddl` on Spark **Parquet** and
`supports_nested_array_ddl` on Spark **Delta** by running the actual ALTER against a live server, then
set **both** the constructor (`dialect.rs`) and the matrix (`multi_backend.md`) to the observed truth,
and remove the P1 exclusion so the conformance suite asserts all cells.

**Critical files.**
- `crates/smelt-dialect/src/dialect.rs:179` (`spark_parquet` `supports_struct_field_ddl`),
  `dialect.rs:154` (`spark_delta` `supports_nested_array_ddl`).
- `crates/smelt-state/src/ddl_spark.rs` — the Spark DDL the flags gate (struct-field add, nested-array
  add); the live check exercises these.
- `docs/specs/multi_backend.md` matrix rows for the two flags + the provisional-cells Known-Divergence
  (retract it once both are pinned).
- `crates/smelt-dialect/tests/capability_conformance.rs` — drop the two `PENDING` exclusions from P1.

**TDD test to write first** (a live-gated test, e.g. `crates/smelt-backend-spark/tests/`):
- `spark_parquet_struct_field_ddl_observed()` and `spark_delta_nested_array_ddl_observed()` — create a
  table of the relevant shape on the live server, attempt the struct-field / nested-array ALTER, and
  record whether it succeeds. Set the constructor flag to the observed boolean; the conformance suite
  (now asserting these cells) must agree with the matrix once the human lands the matrix edit.

**Verification (P2).** With Spark live: the two observed booleans are recorded; `dialect.rs` matches
them; the conformance suite asserts all cells with no exclusions. **The matrix edit is human-gated** —
if the observed truth differs from the current matrix value, set the code, record the needed matrix edit
in §"Blocked phases", and block. When Spark is unset: block "undetermined — re-run with server".

---

### Phase P3: Wire the cross-engine `read_parquet` substitution end-to-end

**Goal.** Make a DuckDB model that references a Spark model resolve to a `read_parquet(...)` against the
Spark model's materialized Parquet **through the run pipeline**, not just via the unit-test-only setter.
W5's surface map found the machinery exists (`PrintContext.cross_engine_refs`, `set_cross_engine_refs`,
the printer substitution) but the runtime step that builds the glob from `find_cross_backend_edges()` +
`materialized_path` and calls the setter appears uncalled outside tests (`execute.rs` only logs the
edges).

**Critical files.**
- `crates/smelt-core/src/graph.rs:167` — `DependencyGraph::find_cross_backend_edges()` (edge source).
- `crates/smelt-runtime/src/execute.rs:121-127` — currently only `tracing::info!`s the edges; this is
  where the `read_parquet` expr must be built (from the dep's `materialized_path` + the matrix's
  exchange rule) and fed to the compiler via `set_cross_engine_refs(...)`.
- `crates/smelt-runtime/src/compile.rs:1820` (`set_cross_engine_refs`, currently uncalled in `src/`),
  `compile.rs:990-992` (the resolution that consumes `cross_engine_refs`).
- `crates/smelt-backend-spark/src/lib.rs:346` (`materialized_path`) — the Parquet glob source.
- Oracle: `multi_backend.md` §"Cross-engine data exchange" (`read_parquet('{warehouse}/{schema}/{model}/**/*.parquet', hive_partitioning = true)`).

**TDD test to write first** (`crates/smelt-cli/tests/cross_engine_parity.rs` or extend
`multi_engine_test.rs`):
- `duckdb_reads_spark_model_via_parquet()` — a two-model pipeline: a Spark model
  (`materialization: table`) and a DuckDB model referencing it; run end-to-end and assert the DuckDB
  model's compiled SQL contains the `read_parquet(...)` substitution **and** the DuckDB model returns
  the Spark model's rows. **Red** if the substitution isn't wired (DuckDB tries a three-part table name
  → resolution error / wrong rows); green after wiring. Requires live Spark to materialize the Parquet;
  skip + record when `SPARK_CONNECT_URL` unset.

**Verification (P3).** With Spark live: the cross-engine reference resolves to `read_parquet` and the
downstream DuckDB model reads the Spark output; the printer cross-engine unit tests (`printer.rs:1957,1996`)
stay green.

---

### Phase P4: Cross-engine Parquet type conformance (decimal precision + timestamp TZ)

**Goal.** Validate that values round-trip correctly across the Spark→DuckDB Parquet boundary for the two
types the spec flags as unvalidated: decimal precision/scale and timestamp timezone. Builds on P3's
wired path.

**Critical files.**
- `crates/smelt-dialect/src/type_conformance.rs` — `wrap_with_type_casts` handles decimal precision
  (test `decimal_with_precision`, `type_conformance.rs:94-106`) but has **no timestamp-timezone**
  handling (the gap). Extend it if the round-trip shows TZ drift.
- `crates/smelt-backend-spark/src/lib.rs` (Parquet write side), DuckDB `read_parquet` (read side).

**TDD test to write first** (`crates/smelt-cli/tests/cross_engine_types_parity.rs`):
- `decimal_precision_roundtrips_spark_to_duckdb()` — a Spark model produces a `DECIMAL(p,s)` column
  materialized to Parquet; a DuckDB model reads it; assert the value and precision/scale are preserved
  (no truncation/rescale).
- `timestamp_tz_roundtrips_spark_to_duckdb()` — a Spark model produces a timestamp column; assert the
  DuckDB read yields the same instant (no silent TZ shift). **Red** if Parquet TZ semantics diverge;
  fix in `type_conformance.rs` (or the read substitution) → green.
- Requires live Spark; skip + record when unset.

**Verification (P4).** With Spark live: both round-trip tests pass; `type_conformance.rs` unit tests
(decimal) stay green.

**Close-out.** When P4 is committed: flip W6's row in `docs/plans/20260628-spark-parity.md` to
`done (<date>)`, update the master Status, commit together. The loop emits `<<MASTER_EXHAUSTED>>`,
surfacing to a human to scaffold **W7 (CI gate + docs + Known-Divergence retractions)**.

---

## Deferred (not in W6)

- Gated CI job (`spark-up.sh` → `cargo test --features spark` → `spark-down.sh`), `CLAUDE.md` command
  entry, `docs-site/` backend pages + backend-support note, and retracting the remaining
  `multi_backend.md` Known-Divergences ("parity not yet verified"; "cross-engine type conformance
  unvalidated" once P3/P4 land; "session init & Arrow loading not yet honored") → **W7**.
- Partition-pruned cross-engine reads (perf, not correctness) → deferred per `multi_backend.md`.
- Databricks as a distinct backend / real Spark materialized views → out of scope (no Databricks
  backend yet).

---

## Blocked phases

### P2 — 2026-06-30

**Partial progress landed.** `supports_struct_field_ddl` on Spark Parquet resolved empirically:
- Live Spark 4.1.x rejects `ALTER TABLE … ADD COLUMNS (struct_col.field TYPE)` with
  `[UNSUPPORTED_FEATURE.TABLE_OPERATION]`. Observed truth = **false**.
- `dialect.rs:spark_parquet().supports_struct_field_ddl` updated `true → false` (now matches matrix `✗`).
- Conformance test PENDING exclusion removed; `every_flag_matches_matrix` now asserts this cell.
- New `crates/smelt-backend-spark/tests/ddl_observed.rs` with both tests committed.

**Remaining blocker:** `supports_nested_array_ddl` on Spark Delta **cannot be verified** — the
test server (`apache/spark:latest`) does not include Delta Lake. The `CREATE TABLE … USING DELTA`
attempt fails with `[DATA_SOURCE_NOT_FOUND] DELTA`.

**Current state (matrix vs code):**
- `supports_struct_field_ddl` / Spark(Parquet): matrix=`✗`, code=`false` → RESOLVED ✓
- `supports_nested_array_ddl` / Spark(Delta): matrix=`✓`, code=`false` → UNRESOLVED

**Options for human:**
1. **Preferred:** Add Delta Lake to the test server (`spark-up.sh` with `--packages io.delta:delta-spark_2.13:4.0.0`) and re-run P2 to empirically verify the Delta cell.
2. **Accept code wins:** Set matrix to `✗` (matching code=false), since OSS Spark ALTER TABLE ADD COLUMNS for array-of-struct fails without Delta column-mapping (`mergeSchema` is the right path). Human must land the matrix edit in `multi_backend.md`.
3. **Accept matrix wins:** Set code to `true` (meaning Delta IS supposed to support it), then verify with a Delta-enabled server.

---

## Verification (wave-level, after P4)

- `cargo build` + `cargo test --quiet 2>&1 | tail -40` green; `cargo clippy --all-targets` and
  `--features smelt-cli/spark` zero warnings.
- `cargo test -p smelt-runtime --test execute_parity` + `cargo test -p smelt-cli --test example_diagnostics` green.
- The capability-conformance suite asserts **every** flag of `::duckdb()`/`::spark_delta()`/
  `::spark_parquet()` against the matrix, with no provisional exclusions remaining; a deliberate
  one-flag drift fails it.
- With `SPARK_CONNECT_URL` live: the two DDL cells reflect observed Spark behavior (code + matrix agree);
  a DuckDB model reads a Spark model via `read_parquet` end-to-end; decimal precision and timestamp TZ
  round-trip across the Parquet boundary.
- `multi_backend.md` matrix and the `BackendCapabilities` constructors agree (the spec §Constraint).
