## Drift Report: schema_evolution

**Spec**: docs/specs/schema_evolution.md (last_reviewed: 2026-05-05)
**Date**: 2026-05-31
**Phase**: C8 (feature sweep)

### Automated checks
- cargo fmt — PASS
- cargo clippy --all-targets — PASS (zero warnings)
- cargo test — PASS (full workspace green)
- example_diagnostics — PASS (87 passed)
- example_workspaces — PASS (27 passed)

### Surface drift
- ✅ Evolution flags `--allow-column-removal` / `--allow-full-refresh` present on `run`/`build` (`main.rs`, threaded into `migration::check_and_migrate`).
- ✅ Column evolution annotations `default:` / `backfill:` consumed (`migration::extract_evolution_maps`, `ColumnMeta`).
- ✅ Stored schema format (`version`, `deployed_at`, `model_hash`, `columns[]`) — `DeployedSchema` (`schema_tracking.rs:115`).
- ✅ `smelt diff` per-model text report + summary line, exits 1 on any change (`commands/diff.rs`).
- ❌ **Spec says `smelt diff [--format text|json]`; the code flag is `--json` (boolean).** `--format json` errors `unexpected argument`. (`main.rs:401-403`) → BUG-037.
- ❌ **JSON `type` values diverge from spec.** Spec Surface lists PascalCase (`"AddColumn"`, `"RemoveColumn"`, `"ChangeType"`, …); code emits snake_case (`"add_column"`, `"remove_column"`, `"change_type"`, …). (`commands/diff.rs:343-441`) → BUG-037.
- ❌ **JSON `risk.migration_action` values diverge.** Spec lists `"NoChange"|"AlterTable"|"FullRefresh"|…`; code emits `"no_change"|"alter_table"|"full_refresh"|…`. (`commands/diff.rs:445-456`) → BUG-037.

### Semantics drift
- ✅ Change classification + safe-widening table — `diff_schemas`, `is_safe_type_widening` (`schema_tracking.rs`), with unit tests in-module and `tests/incremental/schema_evolution.rs` (15+ `check_and_migrate` e2e cases).
- ✅ Migration actions (`NoChange`/`AlterTable`/`RequiresColumnRemovalFlag`/`FullRefresh`/`FullRefreshBlocked`/`TableRewrite`) — `MigrationAction`, `plan_migration_for_backend`.
- ✅ Backend capability matrix + struct_pack — `ddl_duckdb.rs`, `ddl_spark.rs`.
- ✅ Version increments on migration — `check_and_migrate` (`version + 1`); `save_deployed_schema`.
- ❌ **Stored-schema key asymmetry (sub-directory models).** Spec: "smelt writes the deployed schema to `.smelt/schemas/<model_name>.json`" and the diff/cleanup operate on the same key. The run-pipeline save + migration-check paths key by the **db-name** (`db_name_owned()`, e.g. `staging_stg_orders`), but the **stale-schema cleanup** compared against canonical `all_model_names()` (`staging.stg_orders`) and `smelt diff` looked schemas up by the canonical path. Net effect for any model in a sub-directory:
  - stale cleanup deleted the just-saved schema every run → schema evolution silently never triggered (BUG-034);
  - `smelt diff` silently skipped the model (`0 models checked`) and would falsely report a live sub-dir model `REMOVED` (BUG-035);
  - `smelt diff` emitted an invalid 3-part table name in ALTER statements (`main.staging.stg_orders`) (BUG-036).
  All three **fixed** this phase (red-green: `tests/schema_roundtrip.rs::subdir_model_schema_persists_and_diffs`).

### Invariant drift
- ✅ Constraint 1 (offline diff) — `diff` reads `.smelt/schemas/` + runs type inference, no DB connection.
- ✅ Constraint 2/3 (column removal / unsafe change require flags) — `plan_migration_for_backend` gating; now actually reachable for sub-dir models post-fix.
- ✅ Constraint 4 (version increments) — verified above.
- ⚠️ **Run-pipeline parity (CLI ↔ UI):** all schema-evolution logic (`check_and_migrate`, schema save, stale cleanup) lives in `smelt-cli/src/commands/run.rs`; `smelt-runtime` has no schema-evolution code. The UI/`execute_project` path performs no migration, no schema persistence, and no stale cleanup. Not re-logged here (BUG-001-class, run-pipeline-parity seam already in the needs-review queue).
- ⚠️ Schema evolution `check_and_migrate` runs **only** for `PhysicalStrategy::Incremental` models (`run.rs:786`); plain `materialization: table` models get a saved schema (so `diff` works) but never an ALTER migration. Spec change-classification reads as materialization-agnostic. Flagged for manual review; not separately logged (entangled with the Incremental-only migration design).

### Timeless-oracle drift
- ✅ No `Phase [A-Z0-9]` leakage in spec body (only in Known Divergences with plan links and References).

### Freshness
- last_reviewed: 2026-05-05; code paths actively evolving (smelt-state, smelt-cli). Spec body remains accurate for classification/widening/actions. The `--format`/JSON-shape Surface text is stale vs code (BUG-037).
- Verdict: mostly fresh; recommend a spec touch-up for the `smelt diff` JSON/flag Surface (BUG-037).

### Summary
- Drift items: 4 (3 fixed code bugs — sub-dir key asymmetry; 1 needs-review Surface/JSON contract).
- Recommended next step: post-sweep human review of BUG-037 (align spec `smelt diff` Surface to the `--json` + snake_case reality, or change the code).
