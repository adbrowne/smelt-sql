# Phase 3b2 — The partition column's declared type reaches the single literal renderer

## Objective

Close gap 1 soundly in **both** directions under the 2026-09-15 ruling (option (a)): the
referenced partition column's SQL type is threaded to
`smelt_logical::maintenance::emit::partition_literal`, which renders `DATE '…'`/`TIMESTAMP '…'`
against a DATE/TIMESTAMP-typed column and today's bare-quoted string against a declared-VARCHAR
one. This unblocks criteria 2 and 7 (a calendar-axis model runs end-to-end on Trino) without
breaking `examples/web_analytics`'s legitimately-VARCHAR `event_date` on DuckDB — the failure
that blocked 3b. Also lands 3b's independently-discovered `render_time_literal`
symbolic-placeholder fix.

**De-risking insight that makes this tractable where 3b was not:** only the *known-DATE/TIMESTAMP*
arm changes spelling. `Text` and `Undeclared` both render exactly as today, so the threading is
mechanical and behaviour-preserving everywhere the type is not resolvable, and fixture churn is
confined to fixtures whose partition column is actually declared DATE/TIMESTAMP — not the ~19
files 3b had to repair.

## Spec delta (spec-first — the implement step makes this edit before code)

- `docs/specs/incremental_shapes.md` §"Validation rules" rule **8a**, the sentence beginning
  "A partition literal is rendered **in the axis's own domain** everywhere a run emits one":
  restate as axis **and column type**. Integer axis → bare (`7`). Calendar axis → `DATE
  'YYYY-MM-DD'` / `TIMESTAMP 'YYYY-MM-DD HH:MM:SS[.fff]'` when the referenced partition column is
  declared or inferred DATE/TIMESTAMP; a quoted, escaped string (`'2026-01-01'`) when it is
  declared a string type or its type is not resolvable. State the reason in one clause: a
  literal/column type mismatch is refused by a strict engine (Trino) in the typed-column
  direction and by DuckDB in the string-column direction, so the *column*, not the dialect,
  decides — the renderer stays dialect-blind and remains the single owner.
- Sweep for any other statement of the spelling (`rg -n "quoted and escaped" docs/specs/`) and
  edit it in the same commit.

## Tests (red-green)

1. `smelt-logical/tests/emit_statements.rs::partition_literal_types_a_calendar_literal_by_column_type`
   — `(Calendar, Date, "2026-01-01")` → `DATE '2026-01-01'`; `(Calendar, Timestamp, "2026-01-01 00:00:00")`
   and the `T`-separated/fractional forms → `TIMESTAMP '…'`; `(Calendar, Text, "2026-01-01")` →
   `'2026-01-01'`; `(Calendar, Undeclared, …)` → `'2026-01-01'`; `(Integer, *, "7")` → `7`.
2. `…::partition_literal_refuses_a_non_calendar_shaped_value_for_a_typed_column` — a value that
   parses as neither date nor timestamp against a `Date`/`Timestamp` column is `Err` naming the
   value (fail-loud, matching the integer arm); against `Text`/`Undeclared` it stays the escaped
   string (`"it's"` → `'it''s'`, today's assertion preserved).
3. `…::region_predicate_uses_the_column_type` — `Region::for_axis(Calendar, Date, …).predicate(..)`
   renders `col >= DATE '…' AND col < DATE '…'`; the `Text` case is byte-identical to today.
4. `smelt-runtime` (transformer's existing unit-test home):
   `injected_source_filter_types_a_date_column_and_quotes_a_varchar_one` — two `SourceBound`s over
   the same calendar range, one `Date`, one `Text`, produce `DATE '…'` and `'…'` respectively.
5. `…::output_clamp_types_the_models_own_partition_column` — `inject_time_filter` over a model
   whose own partition column infers DATE emits `DATE '…'` in the `_smelt_output_clamp` predicate.
6. `smelt-runtime/src/transformer.rs` unit:
   `render_time_literal_passes_symbolic_window_placeholders_through` — `{{window_start}}` /
   `{{window_end}}` (the tokens `diagnostics::preview::placeholder_range` legitimately feeds this
   path for a no-`--period` preview) render unchanged and do not trip the strict renderer.
   Regression fence for 3b's discovered panic.
7. `smelt-cli/tests/e2e/…` or the existing DuckDB-live statement-parity home:
   `varchar_partition_column_still_compares_on_duckdb` — a real DuckDB run over the
   `examples/web_analytics`-shaped `event_date: VARCHAR` fixture (the exact shape
   `statement_parity/staged_candidate_conditional.rs` exercises) still executes. This is 3b's
   blocker as a standing fence.
8. `smelt-cli/tests/trino_incremental_families.rs::calendar_axis_incremental_model_runs_on_trino`
   — live tier, mirroring 3a's integer-axis test: seed → backfill → steady-state re-run of a
   model whose partition column is a real Iceberg `DATE`, via `smelt run --target trino`. The
   test that proves gap 1 is closed. **If `scripts/trino-env.sh` cannot reach the coordinator,
   emit `<<PHASE_BLOCKED>>` — never skip green.**

## Tasks

1. Make the spec edit above.
2. Add `PartitionColumnType { Date, Timestamp, Text, Undeclared }` to `smelt-logical`'s
   maintenance `emit::types` beside `PartitionAxis`, with a doc comment classifying `Undeclared`
   as the *compatibility* arm (renders as today; it is not an `Unknown`-style silent default
   because it is only reachable where no declaration or inference exists, and it never changes a
   currently-working spelling).
3. Rewrite `partition_literal(axis, column_type, value)` in
   `crates/smelt-logical/src/maintenance/emit/types.rs`: calendar + `Date`/`Timestamp` → shape
   classifier parsing with `chrono` (date → `DATE '…'`, space/`T` separator with optional
   fractional seconds and trailing `Z` → `TIMESTAMP '…'`, otherwise `Err`); calendar +
   `Text`/`Undeclared` → today's escaped quoted string; integer → unchanged. Extend
   `Region::for_axis` the same way. Update the doc comments on both and on
   `crates/smelt-backend/src/types.rs:94`.
4. Carry the type on the existing value types rather than inventing a second channel:
   `SourceBound` gains `partition_col_type`, `PartitionRange` gains `column_type`. Fill them at
   construction — source-side from `SourceColumn::data_type` (`crates/smelt-core/src/sources.rs`,
   already declared data, e.g. `type: VARCHAR`) reached via
   `build_model_source_bounds`'s `dep_ts` map, which widens from
   `(Vec<String>, String)` to also carry the column type.
5. Model-output side: resolve the model's own partition-column type from the **source-derived**
   projection the compile path already infers (`infer_select_column_types`) — never by re-parsing
   printed SQL (the source-derived-projection invariant). Where a caller has no channel to it,
   pass `Undeclared` and leave a one-line comment saying so.
6. Update the three backend call sites (`smelt-backend-{duckdb,spark,bigquery}/src/{lib,sql}.rs`)
   to pass `partition.column_type`; no backend gains a branch of its own.
7. Fold in 3b's fix: a `render_time_literal` wrapper in `crates/smelt-runtime/src/transformer.rs`
   special-casing the two symbolic placeholder tokens before delegating to `partition_literal`,
   used by the two `unwrap_or_else` sites (~140, ~149) so the `debug_assert` fence stops firing on
   a legitimate preview path.
8. Run `cargo test --workspace` and repair fixtures. **Expected to be a small set** — only
   DATE/TIMESTAMP-declared partition columns change spelling. If it is not small, stop and
   record why before mass-editing.
9. Re-check the doc-freshness surfaces only if step 8 changed any rendered SQL they pin
   (`cargo test -p smelt-cli --test tutorial_freshness --test explain_snapshots`); regenerate via
   `examples/web_analytics/tutorial_pages/` templates, never the generated `docs-site/` output.
10. Fix the stale header comment in `crates/smelt-cli/tests/trino_incremental_families.rs`
    claiming Trino has no `MaintenanceDialect` (3c's summary: it predates phase 3).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-logical --test emit_statements --test walk_coverage`
- `cargo test -p smelt-runtime --test partition_axis_windowing --test statement_parity --test dry_run_statements --test execute_parity --test projection_dialect_invariance`
- `cargo test -p smelt-cli --test maintenance_conformance --test transformer_metamorphic`
- Live tier: `bash scripts/trino-up.sh && source scripts/trino-env.sh`, then
  `cargo test -p smelt-cli --test trino_incremental_families` and
  `cargo test -p smelt-backend-trino --test backend_live` (run these **serially**,
  `--test-threads=1` — 3c recorded cross-test schema races under default parallelism); tear down
  with `scripts/trino-down.sh`.

## Commit message

`fix(partitions): render a calendar partition literal typed or quoted by its column's declared type`
