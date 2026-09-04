# crates/smelt-state/CLAUDE.md

Run state persistence — `RunManifest`, `IntervalStore`, `DeployedSchema`, DDL trackers for DuckDB and Spark, and the `FileStore` that reads/writes `.smelt/` on disk.

## How to test

```bash
cargo test -p smelt-state
```

## Gotchas

- **`.smelt/` directory layout is fixed, and partitioned per target.** `FileStore::new(project_dir, target)` writes every run-scoped artifact under `.smelt/targets/<target>/`, so `dev` state and `prod` state never share a file:
  - `.smelt/targets/<target>/runs/{run_id}.json` — one manifest per run
  - `.smelt/targets/<target>/intervals.json` — cumulative interval coverage
  - `.smelt/targets/<target>/landed_deltas.json` — per-source landed-delta intervals
  - `.smelt/targets/<target>/snapshots.json` — fingerprint/environment snapshots
  - `.smelt/targets/<target>/source_postures.json` — per-source append-only posture baselines
  - `.smelt/targets/<target>/source_mutations.json` — per-source `UpstreamMutation` cell dispatch fingerprint baselines
  - `.smelt/targets/<target>/frozen_band_baselines.json` — per-source `contract.frozen_horizon` frozen-band row-count baselines
  - `.smelt/targets/<target>/schemas/{model}.json` — deployed schema snapshots (schema tracking)
  Only `.smelt/meta.json` (layout-version marker) and `.smelt/lock` (the project-wide advisory lock — deliberately *not* per-target; see the doc comment on `FileStore` for why) live at the `.smelt/` root, shared across every target. A missing `meta.json` denotes the legacy pre-partitioning layout (root-level `runs/`, `intervals.json`, etc., no `targets/<target>/` nesting); `FileStore::lock()` migrates it in place, once, under the lock. Never create state files outside `.smelt/`; the `.smelt/` root is gitignored in example workspaces.
- **The reconciliation ledger is engine-resident, not a `.smelt/` file.** `_smelt_ledger` (`src/ddl_duckdb.rs`) lives in the warehouse, transactional with the write it protects (`docs/outcomes/20260904-state-residency/outcome.md`). `src/reconciliation.rs`'s `ReconciliationLedger` is the algebra's pure in-memory reference model that this crate's own tests pin against — no production code persists one. A legacy root-level `reconciliation.json` from a pre-residency `.smelt/` is left untouched by `FileStore`'s legacy-layout migration (inert dead weight, not read).
- **`RunManifest` is serialized to JSON.** Adding required fields (no `Option<>`, no `#[serde(default)]`) is a breaking change for anyone reading historical manifest files. Prefer `Option` or `#[serde(default)]` for new fields.
- **`IntervalStore` uses string date keys** (`"2024-01-01"`), not `NaiveDate`, in its JSON representation. Interval arithmetic (`merge_intervals`, `find_gaps`) operates on `NaiveDate` internally.
- **`ddl_duckdb.rs`, `ddl_spark.rs` and `ddl_bigquery.rs`** generate DDL strings for `CREATE TABLE AS` and schema-migration statements. They are not execution backends — they produce SQL strings that the relevant backend then runs.
- **`generate_run_id()`** (in `src/lib.rs`) produces a timestamped UUID string. Use it for new run manifests rather than generating IDs elsewhere.

## Where things live

- `src/lib.rs` — `RunManifest`, `ModelRunRecord`, `TimeRangeRecord`, `generate_run_id`
- `src/file_store.rs` — `FileStore`; reads/writes `.smelt/`
- `src/intervals.rs` — `IntervalStore`, `ModelIntervals`, `Interval`, `Gap`
- `src/schema_tracking.rs` — `DeployedSchema` for tracking deployed column sets
- `src/ddl_duckdb.rs` — DDL generation for DuckDB schema changes
- `src/ddl_spark.rs` — DDL generation for Spark schema changes; every rule is a fact measured by `scripts/spark-probe-ddl.sh` against a live server, not a reading of the Spark/Delta docs, and the rules are stated for the table smelt creates (`USING DELTA` with no table properties, v1 Parquet) rather than for the format in the abstract. Spark is routed through it for the **whole** diff in `plan_migration_for_backend`, before the per-change loop, because that loop writes the flat cases in DuckDB's dialect for every backend.
- `src/ddl_bigquery.rs` — GoogleSQL DDL generation; every rule is a fact measured by `scripts/bigquery-probe-ddl.sh` against a live warehouse, not a reading of Google's docs (three of them contradict the obvious guess — see the module docs). BigQuery is routed through it for the **whole** diff in `plan_migration_for_backend`, before the per-change loop, because that loop writes the flat cases in DuckDB's dialect for every backend.
- `src/history.rs` — run history queries over saved manifests
