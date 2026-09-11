# Phase 13 summary — the merge ledger is realised on BigQuery

**Row:** 13 · **Criteria served:** 8, 9, 3, 7 · **Status:** done

## What landed

**The GoogleSQL ledger spelling** (`crates/smelt-state/src/ddl_bigquery.rs`, +~200 lines
plus 9 unit tests). All five builders ported: `generate_ledger_table_ddl`,
`generate_ledger_insert_sql`, `generate_ledger_upsert_sql`, `generate_ledger_exists_sql`,
`generate_ledger_recompute_reset_sqls`. Same table name (`crate::ddl_duckdb::
LEDGER_TABLE_NAME`, reused rather than re-declared), same six columns, same four-column
key. Three things are genuinely different and each is asserted verbatim:

- The name is `` `<schema>._smelt_ledger` `` — one backticked path via the existing
  `qualified()`, which is the shape `smelt_backend_bigquery::sql::qualified_name` produces.
  Two-part, resolving against the job's default project, because the driver passes only
  `schema`. A project-prefixed schema (`my-proj.ds`) stays one path; tested.
- `PRIMARY KEY (…) NOT ENFORCED`, with the doc comment saying plainly that it documents row
  identity and refuses nothing.
- The upsert is `MERGE … USING (SELECT <literals>) S ON <4 key equalities> WHEN NOT MATCHED
  THEN INSERT`. `SELECT` rather than `UNNEST([STRUCT(…)])`: phase 12's finding 1b is about a
  row *set*, and this source is always exactly one row — stated in the doc comment.

One thing the plan did not name: **string escaping differs**. `ddl_duckdb::
escape_sql_literal` doubles the quote; `''` does not continue a string in GoogleSQL, and a
backslash IS an escape character there. `ddl_bigquery::escape_string_literal` uses the
backslash form for both, with a test (`string_literals_use_googlesql_backslash_escaping`).
Left unfixed this would have been a silent value corruption for any model name or partition
value carrying either character.

**One dispatch point** (`crates/smelt-state/src/ledger.rs`, new). Five functions keyed on
`SqlDialect`, exhaustive (a new dialect is a compile error), returning
`Result<_, UnsupportedLedgerDialect>`. Spark is an error naming the dialect, never
DuckDB-flavoured SQL — its absence is permanent, not pending. `driver.rs`'s two ledger sites
route through it; `succession/execute.rs`'s two (phase 15's) deliberately do not yet.

**The run-layer gate is derived, not compared.** `maintenance_driver::
realises_merge_ledger(dialect)` (new `maintenance_driver/ledger.rs`) reads
`realisable_state_structures`, exactly as `records_observed_deltas` does. So
`driver.rs:611`'s `MergeLedger` guard is *gone from the census entirely* rather than merely
retired — the census's own module doc calls that the preferred fix, and it means the row can
never drift from the guard again.

**The backend seam.** `BigQueryBackend::execute_write_with_bookkeeping` now overrides the
trait default. The statement list is built by a pure
`sql::write_with_bookkeeping_plan(ensure_sqls, pre_write_sqls, write_sqls) -> Vec<String>`,
which is where the contract is asserted (4 tests): `ensure_sqls` first, each its own job and
outside the transaction; `pre_write_sqls` + the write group inside one `BEGIN TRANSACTION …
COMMIT TRANSACTION`, record before write; every statement `;`-terminated exactly once.

**The row flipped.** `realisable_state_structures(BigQuery)` →
`vec![StateStructure::MergeLedger]`; `SparkSQL` split out to its own `vec![]` arm so the two
absences read differently, which they are.

## Decisions that diverge from, or go beyond, the plan

1. **`ReconciliationLedger` stayed off — the plan's reading is correct.** Confirmed by
   reading `driver.rs:485`'s arm: the `Grade::Additive` refusal is `fold_ledger_delta`
   returning `BackendError::AlreadyReflected`, which on DuckDB is the `PRIMARY KEY`
   violation itself. BigQuery's key is unenforced, so the same statements would silently
   double-count. The guard stays, annotated, with its comment rewritten to say the ledger
   text now exists and what is missing is the enforced refusal, naming row 14. Its three
   builders were routed through the dispatch anyway, so row 14 only has to make the refusal
   correct.

2. **Rollback is explicit, and DDL-inside-the-transaction is unverified.** BigQuery does not
   unwind a script's transaction by itself on a mid-script failure, so the script is wrapped
   in `BEGIN … EXCEPTION WHEN ERROR THEN ROLLBACK TRANSACTION; RAISE USING MESSAGE =
   @@error.message; END` — the error still reaches the caller. What is **not** proven
   offline: the first step's `action_group` is a `CREATE TABLE … AS`, so the write group can
   contain DDL, and whether BigQuery accepts that inside a transaction cannot be settled
   without a warehouse. Phase 16 owns it. If it is refused, the fix is local to
   `write_with_bookkeeping_plan`.

3. **No transaction is opened when there is nothing to bind.** With `pre_write_sqls` empty
   the plan is the trait default's own sequence. The transaction exists to make a
   bookkeeping record and its write atomic; with no record it would only be new, unverified
   behaviour on paths that never asked for it.

4. **`succession/execute.rs` was left alone.** Its two ledger calls sit behind the
   `TombstoneLedger` guard (phase 15). Routing them now would have been scope creep into a
   phase that has to change their semantics anyway.

## Pre-existing tests corrected (never deleted)

| Test | Why it moved |
|---|---|
| `maintenance_availability/realisation.rs::has_emitters` | Was `dialect`-only; now per `(dialect, structure)`. This is the table the plan calls "the table a phase adding a dialect's realisation edits" — it is doing its job. |
| `…::bigquery_and_spark_realise_no_state_structure_today` | Renamed `each_dialect_realises_exactly_the_structures_it_has_today`; Spark's absence is asserted as permanent, BigQuery's as exactly `{MergeLedger}`. |
| `maintenance_availability/succession.rs::a_ledger_less_dialect_realises_no_ledger` | The merge-ledger assertion now applies to Spark only; the reconciliation-ledger one still covers both, with the unenforced-key reason in a comment. |
| `state_guard_census.rs::the_census_is_non_empty_and_covers_the_known_guards` | Four known guards → three; `MergeLedger` joins `ObservedOutputDeltas` in the "must have NO raw guard, because its gate is derived" list. |
| `example_diagnostics/smoke_and_migration.rs::github_activity_no_diagnostics` | The `ColumnScopedMerge` → merge-ledger downgrade no longer fires on the `bigquery` target. Its **absence** is now the documented assertion. |

That last one is the phase's most legible result: a real example workspace stopped
degrading a real cell.

## New tests

- `smelt-state` lib, `ddl_bigquery::ledger_tests` (9): verbatim text for all five builders,
  `NOT ENFORCED` present, `ON CONFLICT` absent, one-row `SELECT` source (no `UNION ALL`),
  GoogleSQL backslash escaping, project-prefixed schema.
- `crates/smelt-state/tests/ledger_dialect.rs` (5, new file): dispatch exhaustive; Spark
  refused by name; BigQuery path carries no `"`-quoted identifier / `ON CONFLICT` /
  `VARCHAR`; DuckDB path byte-identical to the builders it delegates to; non-vacuity — the
  two realising dialects must not produce the same text.
- `smelt-backend-bigquery` `sql::tests` (4): the transaction-boundary contract above.
- `maintenance_availability/realisation.rs::bigquery_keeps_a_merge_ledger_technique_and_still_downgrades_a_keyed_fold`
  — the plan's test 5, both halves in one test.

## Gates

| Gate | Result |
|---|---|
| `bash .claude/scripts/verify-phase.sh` | **ALL GREEN** (fmt, clippy both feature sets, workspace `cargo test`, `example_diagnostics`) |
| `cargo test -p smelt-runtime --test state_guard_census` | 3 passed |
| `cargo test -p smelt-runtime --test availability_seam` | 6 passed |
| `cargo test -p smelt-logical --test maintenance_availability` | 21 passed |
| `cargo test -p smelt-logical --test maintenance_dialect_blindness` | 3 passed |
| `cargo test -p smelt-state --test ledger_dialect` | 5 passed |
| `cargo check -p smelt-cli --features bigquery` | clean |
| `bash .claude/scripts/large-file-check.sh` | OK — no tracked file over baseline, no new file over 1500 (`ddl_bigquery.rs` 607 → 967) |

No live warehouse was reached.

## What phases 14 and 16 inherit

- **Row 14** gets the reconciliation ledger's GoogleSQL statements already built and already
  dispatched. Its whole remaining job is the refusal: `fold_ledger_delta`'s `AlreadyReflected`
  has to come from somewhere other than an unenforced key — the transaction seam this phase
  added is the natural place (read the `exists_sql` and the fold inside one transaction),
  but that is row 14's call to make and to red-green.
- **Row 12** (observed deltas) is unblocked on its transactional half:
  `execute_conditional_write_and_record_observed_delta` delegates to the seam this phase
  overrode, so row 12 needs only the `ddl_bigquery` observed-delta emitters and the row flip.
- **Row 16** must verify two things offline evidence cannot settle: that BigQuery accepts a
  `CREATE TABLE … AS` inside the script's transaction, and that the `MERGE`-as-upsert really
  is a no-op on a repeat window against the live engine.
