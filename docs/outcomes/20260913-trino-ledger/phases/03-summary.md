# Phase 3 summary — `ddl_trino`, measured Iceberg schema-evolution DDL

## Shipped

- `scripts/trino-probe-ddl.sh` — probes type spellings (18 candidates), ADD COLUMN forms,
  nullability toggling, DROP COLUMN (top-level + dotted struct field), `SET DATA TYPE`
  widening (scalar/array/decimal/dotted struct field), `USING`, backfill/rewrite `UPDATE`,
  nested ADD COLUMN, RENAME COLUMN, and schema-mismatched INSERT. Full verbatim output
  captured below.
- `crates/smelt-state/src/ddl_trino/` (`mod.rs`, `types.rs`, `operations.rs`) — `TrinoMigration`,
  `trino_type_sql`, `generate_trino_ddl`, one function per `SchemaOperation` variant, each
  gated by `BackendCapabilities` for the struct/array/column-mapping forms.
- `DdlBackend::Trino { catalog, capabilities }` wired into `plan_migration_for_backend`
  (whole-diff routing, mirroring the Spark/BigQuery arms) and `ddl_backend_for_dialect`
  (`crates/smelt-runtime/src/schema_evolution.rs`) now returns `Ok` for `SqlDialect::Trino`.
- `docs/specs/schema_evolution.md`: Trino column added to the backend capability matrix,
  new §"Trino/Iceberg DDL" subsection, constraint 9 ("Trino never receives another backend's
  DDL"), References entries.
- `crates/smelt-cli/tests/trino_ddl_live.rs` — live-gated (`SMELT_TRINO_URL`), 11 cases
  executed against a real Iceberg table: passed with the tier up.

## Decisions

- **RewriteColumn and AddStructField-with-default are always refused**, not attempted as a
  two-step `SET DATA TYPE` + `UPDATE`: that shape is only sound when the target type is
  already assignment-compatible, which `WidenColumnType` already covers. Keeps the module's
  refusal surface honest rather than half-solving a genuine value rewrite.
- **`RemoveStructField` and nested `WidenNestedType` are DDL, not a refusal** — unlike Spark
  and BigQuery. Iceberg's field-ID column tracking (`supports_column_mapping`) makes a nested
  `DROP COLUMN` safe with no rewrite; measured, not assumed.
- **No column ever gets a persistent `DEFAULT`.** Iceberg refuses the clause table-wide
  (`Default column values are not supported for Iceberg table format version < 3`), so
  `default:` only ever backfills existing rows — sound because every smelt write recomputes
  every declared column from the model's own SQL.
- **Discovered mid-implementation: `ALTER COLUMN "c" DROP NOT NULL` with a quoted column name
  fails `Column '"c"' does not exist` on `trinodb/trino:483`, while every other ALTER COLUMN
  form accepts quoting.** Fixed by emitting that one statement's column name unquoted;
  documented in the module header table, `operations.rs`, and the spec's identifier-quoting
  row. Added a regression note to the module doc rather than silently "fixing" the symptom.
- Capability probes from T1 (`struct_field_ddl`, `alter_column_using`, `nested_array_ddl`,
  `merge_schema_write`, `column_mapping`) were all **confirmed**, not corrected — no
  `smelt-dialect` change needed.
- `DdlBackend::Trino` carries `capabilities: BackendCapabilities`, mirroring Spark, and
  `ddl_for_operation` actually consults it (struct-field-ddl/nested-array/column-mapping
  gates) rather than leaving the field unused — a future non-standard Iceberg catalog with a
  narrower measured capability set degrades correctly instead of sending untested DDL.

## For the next planner

- `large-file-baseline.txt` bumped for `schema_tracking.rs` (4611 → 4662) — the whole-diff
  Trino dispatch block landed in the same file as the Spark/BigQuery arms, matching existing
  structure; not something to "fix" later.
- The map-value-widening key-type placeholder (unbounded `VARCHAR`) is an **existing**
  cross-backend gap `ddl_duckdb` already carries — `ddl_trino` inherits rather than fixes it.
  Worth a dedicated phase across all three generators if a non-VARCHAR-keyed map ever surfaces
  in a real workspace.
- Phase 4 ("Wire the absence") is next: availability resolution should downgrade every
  dependent maintenance cell to its recompute equivalent on Trino. This phase only gave Trino
  the schema-evolution half; the state/ledger absence is still unaddressed.
- `docs/specs/multi_backend.md` capability table was checked and needed no correction (all
  five T1 cells confirmed).

## Gates

- `bash .claude/scripts/verify-phase.sh` — **ALL GREEN** (fmt, clippy both feature sets,
  shellcheck, full `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-state` — 353+2+18+5 passed.
- `cargo test -p smelt-runtime --lib schema_evolution` — 12 passed.
- `cargo test -p smelt-runtime --test statement_parity` — 41 passed (required adding
  `smelt-state/src/ddl_trino/` to the structural scan's directory exclusions, alongside
  `ddl_spark.rs`/`ddl_bigquery/` — same per-dialect-owner exemption).
- With the tier up: `cargo test -p smelt-cli --test trino_ddl_live` — 1 test, 11 live cases
  executed (not skipped).
- `cargo test -p smelt-core --test trino_docs_freshness` — 6 passed.
- `cargo test -p smelt-logical --test state_realisability_docs` — 4 passed.
- `bash .claude/scripts/shellcheck-gate.sh` — PASS (85 scripts, zero findings).
- `bash .claude/scripts/large-file-check.sh` — OK after baseline bump (sign-off note added).
