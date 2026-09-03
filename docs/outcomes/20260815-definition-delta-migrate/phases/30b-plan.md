# Phase 30b plan — schema-evolution DDL: one author per dialect, inside the gate

## Objective

Close the second-author gap phase 30 surfaced: `smelt-state` authors model-table
`ALTER TABLE … ADD/DROP/ALTER COLUMN` text in **two** places — `ddl_duckdb::generate_duckdb_ddl`
and `schema_tracking::generate_migration_sql`'s inline safe-path loop — and neither is covered by
`statement_parity`'s structural scan. Resolve it the way the spec already frames it (schema-evolution
DDL is *not* a maintenance/backbuild statement family; it is multi-dialect and covers struct/nested/
nullability operations the backbuild emitters have no forms for), by declaring `smelt-state`'s
per-dialect `ddl_<dialect>.rs` modules its single owners, collapsing the inline second author into
them, and widening the structural scan to `smelt-state/src` so a *third* author fails the build.
Serves success criterion 9 (all standing gates green, statement single-ownership actually enforced).

## Spec delta (spec-first — the implement step makes these edits)

1. `docs/specs/incremental_models.md` §"Statement emission (single owner)" — the existing clause
   "Non-maintenance SQL (introspection, seed loading, schema-evolution DDL) is outside this rule"
   gains one sentence: schema-evolution DDL is outside the *maintenance/backbuild emitter* rule but
   is itself single-owned per dialect by `smelt-state`'s `ddl_duckdb.rs`/`ddl_spark.rs`/
   `ddl_bigquery.rs` — it is not routed through `backbuild::emit` because those emitters are
   DuckDB-test-grade and have no forms for struct-field, nested-widening, or nullability operations,
   and because `smelt-state` sits below `smelt-logical`. No caller outside those three modules
   composes schema-evolution DDL text.
2. `docs/specs/architecture.md` §"Constraints & Invariants" item 12 — the standing-gate sentence
   names the widened scan: the structural leg also scans `smelt-state/src`, with the three
   `ddl_<dialect>.rs` modules as declared per-dialect owners (excluded, not unscanned).

## Tests (red-green)

- `crates/smelt-runtime/tests/statement_parity.rs::no_maintenance_statement_authoring_outside_the_emitter`
  — add `smelt-state` to the scanned crate list and the three `ddl_<dialect>.rs` modules to the
  declared-owner exclusions. **Red first**: it fails naming `schema_tracking.rs:1470…1593`.
- `crates/smelt-state/src/ddl_duckdb.rs::renderers_are_what_generate_duckdb_ddl_emits` — a direct
  `render_*` call is byte-identical to the corresponding `generate_duckdb_ddl` statement for
  add/drop/alter-type/set-not-null, anchoring the extraction as behaviour-preserving.
- `crates/smelt-state/src/schema_tracking.rs::safe_path_migration_sql_comes_from_ddl_duckdb_renderers`
  — `generate_migration_sql` over a diff with AddColumn (nullable, NOT NULL+default, NOT NULL+backfill),
  RemoveColumn, ChangeType and both ChangeNullability directions is byte-identical to the direct
  `render_*` calls, in order.
- `crates/smelt-state/src/schema_tracking.rs::safe_path_quotes_a_keyword_column_name` — a column named
  `order` renders quoted in the safe path (proves delegation to the quoting renderer, not a copy).

## Tasks

1. Widen the scan (crate list + `EMITTER_MODULE_EXCLUSIONS`, with the per-dialect justification in the
   doc comment) and watch it go red on `schema_tracking.rs`.
2. Extract string-typed renderers in `ddl_duckdb.rs` — `render_add_column(qualified, column, type_sql,
   nullable, default_expr)`, `render_drop_column`, `render_alter_column_type`, `render_set_not_null`,
   `render_drop_not_null`, `render_backfill_update(…, where_null: bool)` — each quoting via
   `quote_identifier`; rewrite `generate_duckdb_ddl` to call them with no output change (its existing
   unit tests are the oracle).
3. Route `schema_tracking::generate_migration_sql`'s safe-path loop (AddColumn / RemoveColumn /
   ChangeType / both ChangeNullability arms) and the nested-change fast path at ~line 1593 through
   those renderers; delete the inline `format!` statement text.
4. Fix up any existing `schema_tracking` assertion whose text changes — the only expected change is
   identifier quoting where the old inline path emitted an unquoted keyword/odd name (a fix, not a
   regression); anything else means the delegation is not behaviour-preserving and must be corrected.
5. Land the two spec edits.

## Verification

- `cargo test -p smelt-runtime --test statement_parity`
- `cargo test -p smelt-state`
- `cargo test -p smelt-runtime --test schema_migration_backfill_atomicity`
- `bash .claude/scripts/verify-phase.sh`

## Commit message

`refactor(schema-evolution): single-owner per-dialect DDL and widen the statement-parity scan to smelt-state`
