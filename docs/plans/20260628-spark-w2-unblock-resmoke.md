# Plan: W2 — Spark unblock (schema-init + Arrow load) + re-smoke

**Parent (master plan)**: `docs/plans/20260628-spark-parity.md` — the **W2** wave. W2 fixes the two
**blocker-class** breaks W1 recorded (BL-1 host-path seed load, BL-2 session schema-init ordering),
then **re-runs the W1 smoke** so the deeper dialect/exec breaks — which BL-2 hid by aborting every
model on first run — finally surface and **extend the recorded break list** that scopes W3.

W2 does **not** fix dialect lowerings (QUALIFY, `::`, `DATE '…'`, …). It removes the two blockers
and lets the smoke discover them. Those become W3.

**Date**: 2026-06-28
**Spec**: `docs/specs/multi_backend.md` — §Surface (`requires_schema_init` flag in the capability
matrix), §Semantics "Session initialization" and "Loading data into a backend", §Constraints
("Data loading carries no host-filesystem assumption"). These are the oracle for P1/P2.
**Spec diff**: landed alongside this plan (human gate, not autonomous) — added the
`requires_schema_init` capability-matrix row, the "Session initialization" and "Loading data into a
backend" §Semantics subsections, and the data-loading §Constraints bullet to `multi_backend.md`.
**Tracking branch**: `worktree-spark`
**Docs**: code/infra-only. The `CLAUDE.md` "Commands" entry and `docs-site/` backend pages land in
W5 (CI gate), not here.

---

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then `docs/specs/multi_backend.md` §Surface + §Semantics "Session initialization" /
"Loading data into a backend" + §Constraints — that is the oracle. Run the next `pending` phase in
the Progress-tracking table (skip `done`/`blocked` rows) using the per-phase routine below. After
the last `pending` phase, flip this sub-plan's row in the master registry
(`docs/plans/20260628-spark-parity.md`) to `done` and commit together. Emit exactly one sentinel:
`<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>`, `<<SUBPLAN_ADVANCED>>`, or `<<MASTER_EXHAUSTED>>`.

**Spark must be live for P1–P3 to be meaningful.** A human runs `scripts/spark-up.sh` once and
exports `SPARK_CONNECT_URL` into the loop's environment (see the master's "Prerequisite"). If
`SPARK_CONNECT_URL` is unset, every Spark assertion **skips** (green) — but then P1/P2 cannot prove
the fix and P3 cannot capture a real break list, so each Spark-dependent phase **blocks** rather
than recording a false green (see per-phase block conditions).

---

## Goal

The two W1 blockers removed and **proven against live Spark**: (1) a fresh-schema first run no
longer dies on `[SCHEMA_NOT_FOUND]`; (2) loading an Arrow batch into Spark no longer dies on
`[PATH_NOT_FOUND]` against a containerized/remote Connect server. Then the W1 smoke re-run, with its
break list **extended** by whatever dialect/exec failures now surface beyond the blockers — the
input a human uses to scaffold W3 (dialect lowerings).

---

## Per-phase routine

1. **Pre-flight.** `cargo build --features spark -p smelt-backend-spark 2>&1 | tail -30` compiles;
   `cargo test --quiet 2>&1 | tail -40` is green (Spark tests skip when `SPARK_CONNECT_URL` unset).
   If red on **unrelated** breakage, treat as a block.
2. **Red-green.** Write the failing test(s) named in the phase first, confirm red (against live
   Spark — the bug only reproduces with `SPARK_CONNECT_URL` set), implement the minimal change,
   confirm green. Implementer pass, then reviewer pass (material findings only).
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
- **Spark-provisioning (P1/P2/P3):** `SPARK_CONNECT_URL` is unset or the server is unreachable, so
  the fix cannot be reproduced/proven (P1/P2) or no real break list can be captured (P3). Block with
  reason "Spark server not provisioned in loop env" — do **not** flip the phase to `done` on a
  skipped (vacuously green) Spark assertion.
- A fix needs a redesign beyond this wave's surface (the two blockers + re-smoke).

---

## Progress tracking

| Phase | Title | Status | Commit | Date |
|-------|-------|--------|--------|------|
| P1 | BL-2: session schema-init (`requires_schema_init` flag + create-before-select ordering) | pending | | |
| P2 | BL-1: load Arrow into Spark via `createDataFrame`, not a host-path Parquet | pending | | |
| P3 | Re-smoke `examples/multi_engine` on live Spark; extend the recorded break list (→ W3) | pending | | |

---

### Phase P1: BL-2 — session schema-init

**Goal.** A fresh-schema first run against live Spark succeeds instead of dying on
`[SCHEMA_NOT_FOUND]`. Root cause (W1 finding): `python/smelt/spark_adapter.py::__init__`
(line ~29) calls `self.spark.catalog.setCurrentDatabase(schema)` at connect time — **before** the
Rust backend's `ensure_schema()` (`crates/smelt-backend-spark/src/lib.rs` ~297–300, which emits
`CREATE DATABASE IF NOT EXISTS catalog.schema` via `sql::create_database()`) ever runs. Selecting a
schema that does not exist is the first statement the session issues, so it fails before any model.

**Spec oracle.** §Semantics "Session initialization": when `requires_schema_init = true`, the
backend creates the target schema during session init **before** selecting it. The flag is `true`
for all backends (matrix row added in the spec diff).

**Critical files.**
- `crates/smelt-dialect/src/dialect.rs` — add `requires_schema_init: bool` to the
  `BackendCapabilities` struct (~28–92); set it in `duckdb()` (~96), `spark_delta()` (~127),
  `spark_parquet()` (~153). All `true` per the matrix.
- `python/smelt/spark_adapter.py` `__init__` (~29) — create the schema
  (`CREATE DATABASE IF NOT EXISTS <catalog>.<schema>`) **before** `setCurrentDatabase(schema)`, OR
  defer `setCurrentDatabase` until after the Rust `ensure_schema()` has run. Pick the ordering that
  keeps `ensure_schema()` the single source of the create (avoid double-emitting).
- `crates/smelt-backend-spark/src/lib.rs` — ensure `ensure_schema()` runs as part of session init
  (gated on `capabilities.requires_schema_init`) and **before** the current-database selection, so
  the very first statement against a fresh warehouse is the `CREATE`, not the `SET`.
- Mirror reference: `crates/smelt-backend-duckdb/src/lib.rs` already does this
  (`new_with_settings()` ~205, `ensure_schema()` ~433) — match the contract, don't regress it.

**TDD test to write first** (gated on `spark_connect_url()`; e.g. extend
`crates/smelt-backend-spark/src/tests.rs` or a new `tests/schema_init.rs`):
- Connect to a **fresh, never-created** schema name (unique per run, e.g. derive from a counter /
  test name — `Date`/random are unavailable in the harness; use a fixed unique literal) and execute
  a trivial `create_table` + `insert` + `select`.
- Red: first run raises `[SCHEMA_NOT_FOUND]` (or the Python adapter raises at connect).
- Green: the model runs; the schema was auto-created during init.
- Add a conformance assertion that `BackendCapabilities::duckdb()/spark_delta()/spark_parquet()`
  each have `requires_schema_init == true` (mirrors the matrix; the full conformance suite is W4 —
  here just assert the one new flag so the spec/code stay in lockstep).

**Verification (P1).** Per-phase routine, plus: with `SPARK_CONNECT_URL` set the fresh-schema test
goes red→green; with it unset the test skips and the suite stays green.

---

### Phase P2: BL-1 — load Arrow via `createDataFrame`

**Goal.** Loading an Arrow batch into Spark works against a containerized/remote Connect server
instead of failing `[PATH_NOT_FOUND]`. Root cause (W1 finding): `SparkBackend::load_table`
(`crates/smelt-backend-spark/src/lib.rs` ~362–443) writes the Arrow batches to a **host** temp
Parquet and calls Python `load_arrow_table(parquet_path, …)`
(`python/smelt/spark_adapter.py` ~60–76), which does `spark.read.parquet(path)` — a host path the
remote JVM cannot see.

**Spec oracle.** §Semantics "Loading data into a backend" + §Constraints "Data loading carries no
host-filesystem assumption": rows are sent through the Connect client (`createDataFrame` from
Arrow), never a host-path read.

**Critical files.**
- `python/smelt/spark_adapter.py` `load_arrow_table` (~60–76) — accept the Arrow data in-band
  (Arrow IPC stream bytes, or a pyarrow `Table`/`RecordBatch` reconstructed from bytes) and build
  the frame via `self.spark.createDataFrame(...)` (or `createDataFrame` over a pandas/pyarrow
  conversion), then `saveAsTable`. No `spark.read.parquet(path)`, no host path argument.
- `crates/smelt-backend-spark/src/lib.rs` `load_table` (~362–443) — serialize the Arrow batches to
  IPC bytes and pass them across the PyO3 boundary instead of writing a temp Parquet + path. Drop
  the host-temp-file plumbing.

**TDD test to write first.** The existing test already names the target contract:
`crates/smelt-backend-spark/tests/load_table.rs::round_trips_via_create_dataframe` (~line 8) — it
round-trips an int32/utf8-with-NULL `RecordBatch` (3 rows, NULL survival) through `load_table`.
- Red (against live Spark, current impl): `[PATH_NOT_FOUND]` because the temp Parquet is host-only.
  If the test currently passes only because host==server filesystem, **strengthen** it to assert no
  host path is used (the spec contract) — e.g. assert via a remote/containerized Connect server, or
  keep the round-trip but confirm the new path emits a `createDataFrame` call. Capture the red first.
- Green: `createDataFrame` path round-trips the batch (3 rows, NULL preserved) with no host path.

**Verification (P2).** Per-phase routine, plus: the gated `load_table` round-trip is green against
the live containerized Connect server; with `SPARK_CONNECT_URL` unset it skips.

---

### Phase P3: Re-smoke + extend the break list

**Goal.** With both blockers gone, re-run the W1 smoke so the pipeline gets **past** session-init
and seed-load and the real dialect/exec breaks surface. Success is "the smoke ran deeper than W1 and
the newly-surfaced breaks are recorded", not "everything is green".

**Critical files.**
- `crates/smelt-cli/tests/spark_smoke.rs` `spark_smoke_multi_engine` (~line 67) — the W1 smoke
  harness (collects `(model, error)` rather than aborting). Re-run it; it now reaches models past
  `stg_sessions`/`int_visitor_daily`. Reuse the W1 `tests/common/mod.rs` helpers
  (`spark_connect_url`, `targets_to_run`, `targets_yaml`, `stage_dual_workspace`, `run_smelt_on`) —
  do not duplicate them.
- Append to this file's **"## Recorded break list"**: one bullet per **newly-surfaced** distinct
  failure (failing SQL construct or exec step, the Spark error, suspected capability flag /
  lowering). Mark BL-1 and BL-2 **resolved (W2)** in place. Mirror a one-line summary into the
  master's Status and, for each new dialect/exec break, ensure the master "## Wave scaffolding
  queue" W3 bullet references it so a human can scaffold W3 from it.

**TDD shape.** Same as W1 P3 — the smoke *is* the test; it collects failures and reports them, only
skipping when `SPARK_CONNECT_URL` is unset. The deliverable is the extended break list, committed.

**P3 block condition.** If `SPARK_CONNECT_URL` is unset/unreachable at run time, **block** (reason
"Spark server not provisioned") — do not flip P3 to `done` with an empty/unchanged break list. A
real extended break list requires live Spark reaching past the blockers.

**Close-out.** When P3's extended break list is recorded and committed: flip W2's row in
`docs/plans/20260628-spark-parity.md` to `done (<date>)`, update the master Status, commit together.
The loop then emits `<<MASTER_EXHAUSTED>>` (no sibling sub-plan), surfacing to a human to scaffold
**W3 (dialect lowerings)** from the extended breaks.

---

## Deferred (to later waves — do not attempt in W2)

- Fixing any dialect lowering the extended break list surfaces (QUALIFY → subquery, `DATE '…'` →
  `to_date`, `::` → `CAST`, `[…]` → `ARRAY(…)`, trailing commas, …) → **W3**.
- Parametrizing the broad CLI test suite over both targets → **W3/W4**.
- Capability-conformance suite (every flag) + cross-engine type validation → **W4**.
- Gated CI job + `CLAUDE.md`/`docs-site` updates → **W5**.

---

## Recorded break list

Carries forward the W1 list; P1/P2 resolve BL-1 and BL-2, P3 appends what the deeper smoke surfaces.

- **BL-1 (seed load / host-path Parquet exchange).** _Resolved in W2·P2 — load via Connect
  `createDataFrame` from Arrow; no host path._ Original symptom: `SparkBackend::load_table` wrote a
  host temp Parquet and asked Spark to `read.parquet(<path>)`, failing `[PATH_NOT_FOUND]` against a
  containerized Connect JVM.
- **BL-2 (no session schema-init / select-before-create ordering).** _Resolved in W2·P1 —
  `requires_schema_init` gates `CREATE SCHEMA IF NOT EXISTS` before `setCurrentDatabase`._ Original
  symptom: `python/smelt/spark_adapter.py::__init__` called `setCurrentDatabase(schema)` before
  `ensure_schema()` ran, so a fresh schema failed `[SCHEMA_NOT_FOUND]` on every model's first run.

_(P3 appends newly-surfaced dialect/exec breaks below.)_

## Blocked phases

_(none yet)_

---

## Verification (wave-level, after P3)

- `cargo test --quiet 2>&1 | tail -40` green with `SPARK_CONNECT_URL` unset (all Spark tests skip).
- With Spark up + `SPARK_CONNECT_URL` set: the P1 fresh-schema test and the P2 `load_table`
  round-trip are green; `spark_smoke` runs deeper than W1 and the extended break list is recorded.
- `cargo clippy --all-targets --features smelt-cli/spark` — zero warnings.
- The `requires_schema_init` matrix row in `multi_backend.md` matches the three constructors.
