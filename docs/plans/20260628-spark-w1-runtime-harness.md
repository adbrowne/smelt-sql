# Plan: W1 — Spark runtime + dual-target harness + smoke

**Parent (master plan)**: `docs/plans/20260628-spark-parity.md` — the **W1** wave. W1 stands up a
real local Spark, builds the reusable dual-target test harness, smoke-tests `examples/multi_engine`
against live Spark, and **records the empirical break list** that scopes W2+. W1 does **not** fix
dialect gaps — it surfaces them.

**Date**: 2026-06-28
**Spec**: `docs/specs/multi_backend.md` — §Surface (capability matrix, `SPARK_CONNECT_URL` gating)
and §Constraints ("Default `cargo test` is backend-agnostic") are the oracle. Do not re-litigate
the matrix.
**Spec diff**: none — `multi_backend.md` was authored alongside this wave; W1 is harness + infra +
discovery, no spec change.
**Tracking branch**: `worktree-spark`
**Docs**: code/infra-only for P1–P3 except the `scripts/` usage note. The `CLAUDE.md` "Commands"
entry and `docs-site/` pages land in W5 (CI gate), not here.

---

## Execution prompt (for a fresh session / autonomy iteration)

Read this file, then `docs/specs/multi_backend.md` §Surface + §Constraints — that is the oracle.
Run the next `pending` phase in the Progress-tracking table (skip `done`/`blocked` rows) using the
per-phase routine below. After the last `pending` phase, flip this sub-plan's row in the master
registry (`docs/plans/20260628-spark-parity.md`) to `done` and commit together. Emit exactly one
sentinel: `<<PHASE_COMPLETE>>`, `<<PHASE_BLOCKED>>`, `<<SUBPLAN_ADVANCED>>`, or
`<<MASTER_EXHAUSTED>>`.

**Spark must be live for P3 to be meaningful.** P1 delivers the scripts; a human runs
`scripts/spark-up.sh` once and exports `SPARK_CONNECT_URL` into the loop's environment (see the
master's "Prerequisite"). If `SPARK_CONNECT_URL` is unset, every Spark assertion **skips** (green)
— fine for P1/P2, but P3 must **block** rather than record an empty break list (see P3 block
condition).

---

## Goal

A reproducible local Spark Connect server (Delta, shared warehouse), a reusable Rust dual-target
test harness that runs an existing CLI integration test against `{DuckDb, Spark}` (Spark
auto-skipping when no server), and a recorded break list from smoking `examples/multi_engine` on
live Spark. The harness is the substrate W3 uses to mirror the bulk of the DuckDB CLI tests; the
break list is the input a human uses to scaffold W2 (dialect lowerings).

---

## Per-phase routine

1. **Pre-flight.** `cargo build --features spark -p smelt-backend-spark 2>&1 | tail -30` compiles;
   `cargo test --quiet 2>&1 | tail -40` is green (Spark tests skip when `SPARK_CONNECT_URL` unset).
   If red on **unrelated** breakage, treat as a block.
2. **Red-green.** Write the failing test(s) named in the phase first, confirm red, implement the
   minimal change, confirm green. Implementer pass, then reviewer pass (material findings only).
3. **Verify.** `cargo fmt --all`; `cargo clippy --all-targets --features smelt-cli/spark` (zero
   warnings); `cargo test --quiet 2>&1 | tail -40` green; the example gate
   `cargo test -p smelt-cli --test example_diagnostics`. For P1, additionally `bash -n` each new
   script.
4. **Record + commit.** Set the table row to `done` + date; commit + push tests + impl + table with
   the phase commit message. Emit `<<PHASE_COMPLETE>>` (or the roll-up sentinel on the last phase).

---

## Block conditions (`<<PHASE_BLOCKED>>` — record and continue)

Set the row to `blocked` + one-line reason; append a dated entry to §"Blocked phases"; restore a
clean committed tree; commit + push; emit `<<PHASE_BLOCKED>>`. Conditions:

- Pre-flight red on unrelated breakage this phase didn't introduce.
- **P3-specific:** `SPARK_CONNECT_URL` is unset or the server is unreachable, so no real break list
  can be captured. Block with reason "Spark server not provisioned in loop env" so a human stands
  it up — do **not** flip W1 to `done` with an empty break list.
- A fix needs a redesign beyond this wave's surface (scripts + `tests/common` + one smoke test).

---

## Progress tracking

| Phase | Title | Status | Commit | Date |
|-------|-------|--------|--------|------|
| P1 | Spark runtime scripts (Connect + shared warehouse + pinned client) | done (2026-06-28) | chore(spark): reproducible Spark Connect runtime scripts | 2026-06-28 |
| P2 | Dual-target test harness (`tests/common`, `TargetKind`, skip-when-no-Spark) | pending | test(cli): dual-target harness over {DuckDb, Spark} | — |
| P3 | Smoke `examples/multi_engine` on live Spark + record break list | pending | test(spark): multi_engine smoke + recorded break list | — |

---

### Phase P1: Spark runtime scripts

**Goal.** Author scripts that stand up a reproducible Spark Connect server with Delta enabled and a
host-shared warehouse, plus a pinned Python client the PyO3 adapter imports. The committed
deliverable is the scripts + a usage note; a human runs `spark-up.sh` to provide the live server
(the loop does not pull Docker images per iteration).

**Critical files.**
- Create `scripts/spark-up.sh` — start the container + Connect server, publish `:15002`, mount a
  host warehouse dir, wait until ready.
- Create `scripts/spark-down.sh` — stop + remove the container.
- Create `scripts/spark-env.sh` — `export SPARK_CONNECT_URL=sc://localhost:15002` and
  `export SMELT_SPARK_WAREHOUSE=<host warehouse dir>` for tests and the loop env.
- Create `scripts/README-spark.md` — one-screen "how to run Spark for parity tests".

**Concrete `spark-up.sh` skeleton** (implementer pins the exact image tag + Delta package that
actually connects; settle empirically — this is the W1 stand-up):
```bash
#!/usr/bin/env bash
set -euo pipefail
IMAGE="${SMELT_SPARK_IMAGE:-apache/spark:4.0.0}"
WAREHOUSE="${SMELT_SPARK_WAREHOUSE:-$(pwd)/.smelt-spark-warehouse}"
DELTA_PKG="${SMELT_DELTA_PKG:-io.delta:delta-spark_2.13:4.0.0}"
NAME="smelt-spark"
mkdir -p "$WAREHOUSE"
docker rm -f "$NAME" >/dev/null 2>&1 || true
docker run -d --name "$NAME" -p 15002:15002 \
  -v "$WAREHOUSE":/opt/spark/work-dir/warehouse \
  "$IMAGE" /opt/spark/sbin/start-connect-server.sh --wait \
    --packages "org.apache.spark:spark-connect_2.13:4.0.0,$DELTA_PKG" \
    --conf spark.sql.extensions=io.delta.sql.DeltaSparkSessionExtension \
    --conf spark.sql.catalog.spark_catalog=org.apache.spark.sql.delta.catalog.DeltaCatalog \
    --conf spark.sql.warehouse.dir=/opt/spark/work-dir/warehouse
# poll until the Connect port answers (implementer: loop on `docker exec ... spark-sql -e 'select 1'` or a gRPC probe)
echo "SPARK_CONNECT_URL=sc://localhost:15002"
echo "warehouse: $WAREHOUSE"
```
Pin the Python client too: `pip install 'pyspark[connect]==4.0.0' 'delta-spark==4.0.0'` into a venv
the adapter sees (document the venv path / `PYTHONPATH` in `README-spark.md`).

**Verification (P1).**
- `bash -n scripts/spark-up.sh scripts/spark-down.sh scripts/spark-env.sh` — clean.
- `cargo build --features spark -p smelt-backend-spark` compiles.
- `cargo test --quiet 2>&1 | tail -40` green with `SPARK_CONNECT_URL` unset (Spark tests skip).
- **Human acceptance (out of loop):** after `bash scripts/spark-up.sh` + `source scripts/spark-env.sh`,
  the existing gated connectivity test passes:
  `SPARK_CONNECT_URL=sc://localhost:15002 cargo test -p smelt-backend-spark --features spark` connects
  (the `spark_connect_url()`-gated tests in `crates/smelt-backend-spark/src/tests.rs` run, not skip).

---

### Phase P2: Dual-target test harness

**Goal.** A shared Rust test module that, given a staged workspace, runs `smelt run` against each
target in `{DuckDb, Spark}` and lets a test assert both produced the same result. DuckDb always
runs; Spark runs only when `SPARK_CONNECT_URL` is set, else skips. This is the substrate W3 reuses.

**Critical files.**
- Create `crates/smelt-cli/tests/common/mod.rs` (shared by `mod common;` in test files):
  - `pub enum TargetKind { DuckDb, Spark }`.
  - `pub fn spark_connect_url() -> Option<String>` — reads `SPARK_CONNECT_URL`.
  - `pub fn targets_to_run() -> Vec<TargetKind>` — `[DuckDb]`, plus `Spark` when
    `spark_connect_url().is_some()`.
  - `pub fn targets_yaml(kind, warehouse_dir) -> (String /*target name*/, String /*YAML block*/)` —
    emits the `targets:` entry: DuckDb → `type: duckdb / database: target/dev.duckdb / schema: main`;
    Spark → `type: spark / connect_url: <url> / catalog: spark_catalog / schema: smelt_w1 /
    warehouse: <dir> / format: delta`.
  - `pub fn stage_dual_workspace(tmp, name, models, warehouse_dir) -> PathBuf` — writes a `smelt.yml`
    containing **both** target blocks (mirroring `run_command_end_to_end.rs::write_smelt_yml`/
    `stage_workspace`) and the model files.
  - `pub fn run_smelt_on(project_dir, target_name, extra_args) -> std::process::Output` — invokes
    `CARGO_BIN_EXE_smelt` with `run --project-dir <dir> --target <target_name>` (confirm the CLI's
    target-selector flag name in P2 step 0 — it is most likely `--target`/`-t`; grep
    `crates/smelt-cli/src/` for the `run` arg parser and use the real flag).
- Create `crates/smelt-cli/tests/dual_target_harness.rs` — the harness self-test (`mod common;`).

**Step 0 (discovery, do first).** `rg -n "target" crates/smelt-cli/src/commands/run.rs` and the CLI
arg parser to confirm the flag that selects a named target. Wire `run_smelt_on` to the real flag.

**TDD tests to write first** (`crates/smelt-cli/tests/dual_target_harness.rs`):
```rust
mod common;
use common::{run_smelt_on, stage_dual_workspace, targets_to_run, TargetKind};
use tempfile::TempDir;

#[test]
fn harness_runs_trivial_model_on_every_available_target() {
    let tmp = TempDir::new().unwrap();
    let warehouse = tmp.path().join("warehouse");
    let models = &[("one.sql", "select 1 as x")];
    let root = stage_dual_workspace(&tmp, "w1_harness", models, &warehouse);

    for kind in targets_to_run() {
        let target_name = match kind { TargetKind::DuckDb => "dev", TargetKind::Spark => "spark" };
        let out = run_smelt_on(&root, target_name, &[]);
        assert!(out.status.success(),
            "smelt run failed on {target_name}: {}", String::from_utf8_lossy(&out.stderr));
    }
}
```
- Red: `mod common;` does not exist → won't compile.
- Green: implement `tests/common/mod.rs`; DuckDb path exits 0; Spark path exits 0 when
  `SPARK_CONNECT_URL` set, and `targets_to_run()` omits Spark (test still green) when unset.

**Verification (P2).** Per-phase routine, plus: with `SPARK_CONNECT_URL` unset the self-test runs
only DuckDb and passes; with it set (human, Spark up) it runs both and passes.

---

### Phase P3: Smoke `examples/multi_engine` + record break list

**Goal.** Run a real Spark-touching pipeline end to end against the live server and **record every
failure** as the break list that scopes W2+. Success is "smoke ran against live Spark and the break
list is recorded", not "everything is green".

**Critical files.**
- Create `crates/smelt-cli/tests/spark_smoke.rs` (`mod common;`, gated on `spark_connect_url()`):
  build `examples/multi_engine` (Spark staging/aggregation → DuckDB metrics) end to end. Prefer
  driving the existing example via `smelt run` with both targets configured; if the example's
  `smelt.yml` already pins per-model targets, point `--project-dir` at a copy with `connect_url` /
  `warehouse` injected from the harness. Assert the final DuckDB metrics table matches a DuckDB-only
  baseline run (row count + a checksum column), capturing any Spark-side build error instead of
  panicking.
- Append a new section **"## Recorded break list"** to THIS file: one bullet per distinct failure
  (failing SQL construct or exec step, the Spark error, and the suspected capability flag / lowering),
  and mirror a one-line summary into the master's Status.

**TDD shape.** The "test" here is the smoke harness itself. Write it to **collect** failures (run
each model, push `(model, error)` into a vec) rather than abort on first error, then assert at the
end and print the collected list. When `SPARK_CONNECT_URL` is unset the test returns early (skips).

**P3 block condition (important).** If `SPARK_CONNECT_URL` is unset or the server is unreachable at
run time, **block** (reason: "Spark server not provisioned") — do not flip W1 to `done` with an
empty break list. A real break list requires live Spark.

**Close-out.** When P3's break list is recorded and committed: flip W1's row in
`docs/plans/20260628-spark-parity.md` to `done (<date>)`, update the master Status, commit together.
The loop then emits `<<SUBPLAN_ADVANCED>>` / `<<MASTER_EXHAUSTED>>` (no sibling sub-plan), surfacing
to a human to scaffold W2 from the recorded breaks.

---

## Deferred (to later waves — do not attempt in W1)

- Fixing any dialect lowering the break list surfaces → **W2**.
- Parametrizing the broad CLI test suite over both targets → **W3**.
- Capability-conformance suite + cross-engine type validation → **W4**.
- Gated CI job + `CLAUDE.md`/`docs-site` updates → **W5**.

---

## Recorded break list

Empirical failures found running real Spark against the live Connect server. Each is a
candidate phase for a human-scaffolded W-wave (see master "## Wave scaffolding queue"). P3 grows
this list; P1 already surfaced one.

- **BL-1 (seed load / Parquet exchange path assumes shared filesystem).** Found in P1 running
  `cargo test -p smelt-backend-spark --test load_table` (`round_trips_via_create_dataframe`).
  `SparkBackend::load_table` writes the Arrow batches to a **host** temp Parquet
  (`/tmp/.tmpXXXX`) and asks Spark to `read.parquet(<path>)`. With Spark in a container (or any
  remote Connect server), that host path is invisible to the JVM →
  `AnalysisException [PATH_NOT_FOUND] Path does not exist: file:/tmp/.tmpICEE6N`. The 8 core
  DDL/DML integration tests (`execute_sql`, `create_table_and_query`, `insert_into`,
  `merge_into`, `delete_partitions`, `create_view`, `execute_model`, dialect/capabilities) all
  **pass** — only the host-temp-file seed path breaks. **Fix direction (W-wave):** load Arrow via
  Spark Connect `createDataFrame` (no path), or stage the temp Parquet inside the shared
  `warehouse` mount that both host and container see. `python/smelt/spark_adapter.py`
  `load_arrow_table` + `crates/smelt-backend-spark/src/lib.rs` `load_table`.

## Blocked phases

_(none yet)_

---

## Verification (wave-level, after P3)

- `cargo test --quiet 2>&1 | tail -40` green with `SPARK_CONNECT_URL` unset (all Spark tests skip).
- With Spark up + `SPARK_CONNECT_URL` set: `tests/common` harness self-test runs both targets green;
  `spark_smoke` runs and the break list is recorded in this file.
- `scripts/spark-up.sh` brings up a Connect server the gated `smelt-backend-spark` connectivity test
  reaches.
- `cargo clippy --all-targets --features smelt-cli/spark` — zero warnings.
