# Phase 30b summary — schema-evolution DDL: one author per dialect, inside the gate

**Shipped:**
- `crates/smelt-state/src/ddl_duckdb.rs`: six new single-owner renderers (`render_add_column`,
  `render_drop_column`, `render_alter_column_type`, `render_set_not_null`,
  `render_drop_not_null`, `render_backfill_update`), each quoting via `quote_identifier`.
  `generate_duckdb_ddl` now calls them instead of building `ALTER TABLE`/`UPDATE` text inline.
- `crates/smelt-state/src/schema_tracking.rs::plan_migration_for_backend`'s DuckDB safe-path
  loop (AddColumn/RemoveColumn/ChangeType/both ChangeNullability arms, plus the nested-change
  `ALTER COLUMN TYPE` fast path) now delegates to those same renderers instead of its own
  `format!` calls — the second author is gone.
- `crates/smelt-runtime/tests/statement_parity.rs`: scan widened to `smelt-state/src`, with
  `ddl_duckdb.rs`/`ddl_spark.rs`/`ddl_bigquery.rs` declared per-dialect owner exclusions.
- Spec edits: `docs/specs/incremental_models.md` §"Statement emission (single owner)" states
  schema-evolution DDL's single ownership explicitly; `docs/specs/architecture.md` item 12
  documents the widened structural scan and its exclusions.
- Tests: `ddl_duckdb.rs::renderers_are_what_generate_duckdb_ddl_emits`,
  `schema_tracking.rs::safe_path_migration_sql_comes_from_ddl_duckdb_renderers`,
  `schema_tracking.rs::safe_path_quotes_a_keyword_column_name`.

**Decisions:**
- Kept `RewriteColumn`'s `... TYPE ... USING ...` shape and the dot-notation struct-field
  helpers inline in `ddl_duckdb.rs` — the plan's six named renderers cover exactly the shapes
  `schema_tracking`'s safe path duplicated; the USING/dot-path forms have no second author to
  close, so extracting them would be scope creep.
- The safe path's `AddColumn` arm now quotes column names via delegation (previously raw
  interpolation) — a behaviour fix, not a regression, matching the plan's expectation; verified
  by the new `safe_path_quotes_a_keyword_column_name` test and confirmed the full `smelt-state`
  suite (296 tests) still passes unchanged otherwise.

**For the next planner:**
- No new residue surfaced. The `RewriteColumn`/struct-pack/list-transform machinery and the
  Spark/BigQuery generators remain single-owned per their own modules already, untouched here.
- Nothing deferred out of this phase's scope.

**Gates:**
- `cargo test -p smelt-runtime --test statement_parity` — PASS (32 tests)
- `cargo test -p smelt-state` — PASS (296 + 2 + 6 tests)
- `cargo test -p smelt-runtime --test schema_migration_backfill_atomicity` — PASS (3 tests)
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  workspace test, example_diagnostics)
