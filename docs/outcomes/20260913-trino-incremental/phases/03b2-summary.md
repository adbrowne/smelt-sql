# Phase 3b2 summary — The partition column's declared type reaches the single literal renderer

**Shipped:**
- `PartitionColumnType { Date, Timestamp, Text, Undeclared }` in
  `crates/smelt-logical/src/maintenance/emit/types.rs`, plus `partition_column_type_for_type`
  (classifies a `smelt_types::DataType`) and a rewritten `partition_literal(axis, column_type,
  value)` — calendar axis now renders `DATE '…'`/`TIMESTAMP '…'` for a `Date`/`Timestamp` column,
  unchanged quoted-string for `Text`/`Undeclared`. `Region`/`Region::for_axis`/`Region::predicate`
  thread the new parameter.
- `SqlCompiler::resolve_partition_column_type` (`crates/smelt-runtime/src/compile.rs`) — the
  single place a caller resolves a partition column's type, guaranteed consistent with
  `apply_type_casts`'s own physical-type decision (see Decisions below).
- `PartitionRange`/`TimeRange`/`SourceBound` all gained a `column_type` field, threaded through
  every backend (`duckdb`/`spark`/`bigquery`/`trino`) and through `smelt-runtime`'s transformer,
  execute/project, execute/window, dry_run, and maintenance_driver modules.
- `render_time_literal` (`transformer.rs`) — passes the two symbolic `{{window_start}}`/
  `{{window_end}}` placeholders through unchanged rather than tripping the strict typed-column
  parse (3b's discovered panic).
- Source YAML column types (`SourceColumn::data_type`) now reach per-source pushdown filters via
  `source_partition_column_type` (`execute/sources.rs`).
- Live test: `calendar_axis_incremental_model_runs_on_trino`
  (`crates/smelt-cli/tests/trino_incremental_families.rs`) — a real `DATE`-typed Iceberg
  partition column, seed → backfill → steady-state on Trino. Gap 1 is closed.
- Spec: `docs/specs/incremental_shapes.md` rule 8a restated for column-type-decided spelling.

**Decisions:** (full detail in `outcome.md`'s Decision log)
- `column_type` must be resolved through the same projection `apply_type_casts` uses
  (`SqlCompiler::resolve_partition_column_type`), never through `resolved_model_schema` — the two
  can genuinely disagree (found live via `examples/web_analytics`). The resolution call additionally
  runs a *shape probe* through `inject_source_filters` first, because the wrapping it performs
  changes how the outer projection resolves a passed-through column's type.
- `IncrementalPlan::column_type` was removed (dead after the above) rather than left as an unused
  second source of truth.

**For the next planner:**
- The root `smelt-db` `derive_projection`/`infer_expression_type` divergence (a subquery-wrapped
  `FROM` losing an upstream model's real column type for some shapes) is NOT fixed — only
  neutralized by routing every consumer through one resolution. Worth its own investigation;
  it may affect other `apply_type_casts` consumers.
- Row `3d`'s deferred live legs (append/whole-row-`MERGE` families through `execute_project`,
  `statement_parity`'s Trino leg) and `3e`'s test-isolation work are unaffected and still open.
- `.claude/large-file-baseline.txt` updated (`--update`) for four files that grew from this
  phase's real threading (`compile.rs`, `cumulative.rs`, `execute/project/mod.rs`,
  `transformer.rs`) plus two test files; no reviewer available in this headless loop, noted here
  as the sign-off record.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy zero-warnings both feature
  sets, shellcheck, full workspace `cargo test`, `example_diagnostics`).
- `cargo test -p smelt-logical --test emit_statements --test walk_coverage` — pass.
- `cargo test -p smelt-runtime --test partition_axis_windowing --test statement_parity --test
  dry_run_statements --test execute_parity --test projection_dialect_invariance` — pass.
- `cargo test -p smelt-cli --test maintenance_conformance --test transformer_metamorphic` — pass.
- Live tier (`scripts/trino-up.sh` / `trino-env.sh` / `trino-down.sh`):
  `cargo test -p smelt-cli --test trino_incremental_families -- --test-threads=1` — 3 passed
  (including the new `calendar_axis_incremental_model_runs_on_trino`).
  `cargo test -p smelt-backend-trino --test backend_live -- --test-threads=1` — 15 passed.
