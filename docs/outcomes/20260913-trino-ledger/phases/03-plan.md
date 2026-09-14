# Phase 3 plan — schema-evolution DDL: `ddl_trino/`, measured against a live Iceberg table

## Objective

Give Trino the half of `smelt-state` it *does* get: every `SchemaOperation` maps to a
Trino/Iceberg statement or to an honest full refresh, with every row of the mapping table
established by executing the form against a live Iceberg table rather than read from
documentation — the discipline `ddl_spark.rs` records in its own header. Lands as a module
directory `crates/smelt-state/src/ddl_trino/`, reachable only through `DdlBackend`, and confirms
or corrects T1's five schema-related capability cells back into the spec in the same commit.
Advances success criteria **6** and **7**.

## Spec delta (spec-first — make these edits before the code)

- `docs/specs/schema_evolution.md` §"Backend capability matrix": add a **Trino + Iceberg** column
  to the operation table, filled from the probe's answers (not from Delta's column). Extend the
  "Every ✗ … resolves to `FullRefreshBlocked`" paragraphs with the Trino sentence.
- `docs/specs/schema_evolution.md`: new subsection **"Trino/Iceberg DDL"** after §"GoogleSQL DDL
  for BigQuery", stating the differences that make another backend's generator wrong here — no
  `::` cast, no `ALTER COLUMN … USING`, unbounded `VARCHAR`, `TIMESTAMP(6)`, `ROW(...)`/
  `ARRAY(...)`/`MAP(...)` spellings, `ADD COLUMN` cannot carry `NOT NULL` or `DEFAULT` — each
  attributed to the measured server answer. Add the module + probe script to §References, and a
  numbered constraint mirroring items 7/8 ("Trino never receives another backend's DDL").
- `docs/specs/multi_backend.md` §"Capability matrix": only if the probe contradicts a measured
  cell — correct `supports_struct_field_ddl`, `supports_nested_array_ddl`,
  `supports_alter_column_using`, `supports_column_mapping` or `supports_merge_schema_write` in the
  same commit as the constructor, with the server's words in the comment.

## Tests

- `ddl_trino::types::trino_type_sql` unit tests — every `DataType` renders the spelling the server
  accepted; a type with no Trino spelling returns `Err` carrying a reason, never a fallback.
- `ddl_trino` unit tests, one per `SchemaOperation` variant — add nullable column, add with
  `default:`, add `NOT NULL`, `RemoveColumn`, `WidenColumnType`, `ChangeNullability` both
  directions, `AddStructField`, `RemoveStructField`, `WidenNestedType`, `BackfillColumn`,
  `RewriteColumn` — each asserting the exact statement list or the `FullRefreshRequired` reason
  naming the column and the Trino limitation.
- `schema_evolution::ddl_backend_for_dialect` — `SqlDialect::Trino` now returns
  `Ok(DdlBackend::Trino { .. })`; the previous `UnsupportedDdlDialect` assertion inverts.
- `plan_migration` with `DdlBackend::Trino` routes the **whole** diff through the Trino generator
  — a DuckDB-spelled `ALTER COLUMN … TYPE` / `::` cast never appears in its output (the
  constraint-7/8 analogue).
- `crates/smelt-cli/tests/trino_ddl_live.rs` (new, gated on `SMELT_TRINO_URL`, skips green when
  unset): for each mapping-table row that claims a statement, create a fresh Iceberg table, run
  the generator's statements against the live coordinator, and assert the server accepted them and
  that `DESCRIBE`/a read-back shows the intended schema. A row the table claims as a full refresh
  asserts the *opposite*: the corresponding raw statement is refused, with the error text recorded
  in the module header comment.
- `crates/smelt-backend-trino/tests/capability_probes.rs` — extend only if a cell is corrected.

## Tasks

1. Write `scripts/trino-probe-ddl.sh` in the shape of `scripts/spark-probe-ddl.sh` and
   `scripts/trino-probe-state.sh` (reuse the latter's `run_stmt`, `CREATED_TABLES` cleanup trap and
   verbatim-error printing; no transaction threading is needed here). One fresh table per case;
   cases cover every `SchemaOperation` form plus the five capability cells, and each type spelling
   (`VARCHAR`, `TIMESTAMP(6)`, `DECIMAL(p,s)`, `ROW`, `ARRAY`, `MAP`).
2. `bash scripts/trino-up.sh && source scripts/trino-env.sh && bash scripts/trino-probe-ddl.sh`;
   capture the full output into `phases/03-summary.md`. **If the coordinator is unreachable, emit
   `<<PHASE_BLOCKED>>` — never write a mapping table from documentation and never let the live
   test skip green as if it passed.**
3. Make the spec edits above from the probe's answers.
4. Create `crates/smelt-state/src/ddl_trino/` as three files, split before it sprawls (criterion 7;
   peers are 1,708 and 1,883 lines and `.claude/large-file-baseline.txt` is a ratchet):
   `mod.rs` (header mapping table + `TrinoMigration` enum + `generate_trino_ddl` dispatch),
   `types.rs` (`trino_type_sql`, identifier quoting with `"`, `catalog.schema.table` qualification),
   `operations.rs` (per-`SchemaOperation` classification). Register in `lib.rs`.
5. Add `DdlBackend::Trino { catalog, capabilities }` and route the whole diff to
   `generate_trino_ddl` in `plan_migration`, in the same shape as the Spark and BigQuery arms.
6. Make `ddl_backend_for_dialect` return it. Keep the `Result` signature and `UnsupportedDdlDialect`
   (a `pub` type reserved for a future dialect; callers already handle it) and reword its doc
   comment, which currently names Trino as the live case.
7. Run the live test with the tier up; record pass counts in the summary. Tear down with
   `bash scripts/trino-down.sh`.
8. Write `phases/03-summary.md`: the verbatim probe output, the final mapping table, which of the
   five capability cells were confirmed vs corrected, and any defect noticed in a peer `ddl_*`
   module (recorded and handed on, per §Out of scope — not fixed here).

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-state` and `cargo test -p smelt-runtime --lib schema_evolution`
- With the tier up: `cargo test -p smelt-cli --test trino_ddl_live` — must report the executed
  cases, not "0 passed / skipped"
- `cargo test -p smelt-core --test trino_docs_freshness`,
  `cargo test -p smelt-logical --test state_realisability_docs`, and the schema-evolution doc
  freshness gate if one exists
- `bash .claude/scripts/shellcheck-gate.sh` for the new script
- `bash .claude/scripts/large-file-check.sh` — no baseline bumped

## Commit message

`feat(state): ddl_trino — measured Iceberg schema-evolution DDL as a module directory`
