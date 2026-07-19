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
  - `.smelt/targets/<target>/reconciliation.json` — reconciliation ledger
  - `.smelt/targets/<target>/landed_deltas.json` — per-source landed-delta intervals
  - `.smelt/targets/<target>/snapshots.json` — fingerprint/environment snapshots
  - `.smelt/targets/<target>/schemas/{model}.json` — deployed schema snapshots (schema tracking)
  Only `.smelt/meta.json` (layout-version marker) and `.smelt/lock` (the project-wide advisory lock — deliberately *not* per-target; see the doc comment on `FileStore` for why) live at the `.smelt/` root, shared across every target. A missing `meta.json` denotes the legacy pre-partitioning layout (root-level `runs/`, `intervals.json`, etc., no `targets/<target>/` nesting); `FileStore::lock()` migrates it in place, once, under the lock. Never create state files outside `.smelt/`; the `.smelt/` root is gitignored in example workspaces.
- **`RunManifest` is serialized to JSON.** Adding required fields (no `Option<>`, no `#[serde(default)]`) is a breaking change for anyone reading historical manifest files. Prefer `Option` or `#[serde(default)]` for new fields.
- **`IntervalStore` uses string date keys** (`"2024-01-01"`), not `NaiveDate`, in its JSON representation. Interval arithmetic (`merge_intervals`, `find_gaps`) operates on `NaiveDate` internally.
- **`ddl_duckdb.rs` and `ddl_spark.rs`** generate DDL strings for `CREATE TABLE AS` and schema-migration statements. They are not execution backends — they produce SQL strings that the relevant backend then runs.
- **`generate_run_id()`** (in `src/lib.rs`) produces a timestamped UUID string. Use it for new run manifests rather than generating IDs elsewhere.

## Where things live

- `src/lib.rs` — `RunManifest`, `ModelRunRecord`, `TimeRangeRecord`, `generate_run_id`
- `src/file_store.rs` — `FileStore`; reads/writes `.smelt/`
- `src/intervals.rs` — `IntervalStore`, `ModelIntervals`, `Interval`, `Gap`
- `src/schema_tracking.rs` — `DeployedSchema` for tracking deployed column sets
- `src/ddl_duckdb.rs` — DDL generation for DuckDB schema changes
- `src/ddl_spark.rs` — DDL generation for Spark schema changes
- `src/history.rs` — run history queries over saved manifests
