# Phase 12 summary — observed deltas are realised on BigQuery

**Row:** 12 · **Criteria served:** 8, 9, 3, 7 · **Status:** done

## Two of the row's three clauses were already satisfied — verified, not rebuilt

- **No `record_observed_delta_with_write` override was needed or added.** That method is
  `Backend::execute_conditional_write_and_record_observed_delta`
  (`crates/smelt-backend/src/lib.rs:697-711`), and it is a thin delegation to
  `execute_write_with_bookkeeping`, which phase 13 already overrode on BigQuery
  (`crates/smelt-backend-bigquery/src/lib.rs:604`). Confirmed by reading both; the delegation
  needs nothing. Asserted instead of re-implemented: the new
  `keyed_fold_suppressed_recording_is_realised_on_bigquery` drives the real driver through the
  seam and reads the statements that come out.
- **There was no driver guard to delete.** Phase 11 had already replaced all three T5 write
  sites and the read site with the derived `records_observed_deltas(dialect)` predicate, so
  flipping `realisable_state_structures` *is* the switch. `state_guard_census` is unchanged (3
  passed), exactly as the plan predicted.
- **Nor did `observed_delta/degradation.rs`'s degradation test need inverting.** Phase 11 had
  already retargeted it at SparkSQL, whose absence is permanent. It is untouched in substance;
  what it gained is a BigQuery twin, so the pair is now "one dialect skips because it cannot,
  the other records because it can" — each keeping the other non-vacuous.

## §0 — the defect phase 14 handed over

`sql::write_with_bookkeeping_plan` put the write group inside its transaction unconditionally,
and BigQuery does not permit DDL on a permanent entity inside one. On a first run the
`Grade::Idempotent` path's write group *is* a `CREATE TABLE … AS`, so phase 13's merge-ledger
realisation would have been rejected by the engine on exactly the run that creates the table.

**Resolution: degrade the atomicity, reorder deliberately, and report it** — not the refusal
option. The refusal was tempting for symmetry with phase 14, but the two cases are not alike:
phase 14's refusal protects a *correctness* guarantee (an unrefused repeat fold double-counts),
whereas this record is bookkeeping, and refusing a first run over bookkeeping would cost a
capability for nothing. The degradation is admissible because of two specific facts, both
written into the doc comment and the spec:

- **Record-before-write is vacuous when the write creates the target.** The ordering exists
  because a record reads the target's pre-write state; a `CREATE TABLE … AS` has no pre-write
  state, and the record's own query would reference a table that does not exist. So the record
  runs second, losing nothing.
- **The surviving exposure is the harmless direction.** A crash between them leaves the table
  created and the window unrecorded → a redundant re-run. The reverse (a record claiming a
  write that never happened) is what could mislead a later run, and this ordering makes it
  impossible.

`write_with_bookkeeping_plan` now returns `BookkeepingPlan { statements, atomicity }`
(`sql.rs:258,300,309`); `BookkeepingAtomicity::NonAtomicCreatingWrite` is the recorded
degradation, and `BigQueryBackend::execute_write_with_bookkeeping` (`lib.rs:616`) emits it at
`warn!` — the level the twin observed-delta skip sites use, after a live run proved `debug!`
invisible to an operator. Detection (`creates_a_permanent_entity`, `sql.rs:329`) is restricted
to a leading `CREATE` (excluding `TEMP`/`TEMPORARY`), because `create_group` is the only DDL the
driver ever puts in a write group. Three tests: the degradation, its non-vacuity (a DML write
group still gets the full transaction), and the temp/DML classifier.

## The three load-bearing checks

1. **`ARRAY_AGG(DISTINCT x IGNORE NULLS)` — confirmed load-bearing, and it needed a fourth
   thing the plan did not name.** GoogleSQL has no `FILTER (WHERE …)`, and `ARRAY_AGG` raises
   on a NULL element rather than yielding a NULL array, so `IGNORE NULLS` is what keeps an
   unmatched key from failing the run. `ARRAY_AGG` over zero rows *is* `NULL` (same as DuckDB),
   so the `COALESCE` is kept — deliberately, not as belt-and-braces — and the empty literal is
   `ARRAY<STRING>[]`, since a bare `[]` has no element type to unify against the aggregate's.
   **The fourth thing: a `CAST(… AS STRING)` around each element.** DuckDB silently casts an
   `INTEGER[]` into a `VARCHAR[]` column; GoogleSQL coerces no array element type on write —
   and `changed_keys_select` emits a literal `NULL AS delta_partition` (an INT64-typed NULL)
   for a model with no partition axis (`column_scoped.rs:304`), so
   `COALESCE(ARRAY_AGG(delta_partition …), ARRAY<STRING>[])` would have been a type error on
   *every* bare keyed model. Found by reading the caller, not by porting the DuckDB text.
2. **NULL-array-vs-empty-array cannot reach the empty-vs-absent rule.** BigQuery genuinely
   cannot distinguish them — a NULL written to an `ARRAY` column reads back empty — but *absent*
   means **no row for the window**, not a NULL column. The upsert's source is one un-grouped
   aggregate `SELECT`, so it writes exactly one row per recorded window even over zero input
   rows; the read filters on the window key alone; `read_observed_delta` returns `None` iff the
   row count is zero. Stated in `ddl_bigquery/observed_delta.rs`'s module doc and on
   `generate_observed_delta_select_sql`, and pinned by
   `ledger_dialect::on_bigquery_empty_and_absent_are_separated_by_row_presence`. A consequence:
   the two `ARRAY<STRING>` columns are declared **without** `NOT NULL` — BigQuery cannot store
   a NULL array anyway, so the constraint is at best redundant and at worst a live-only DDL
   rejection, and nothing depends on it.
3. **The Arrow list decode — hardened rather than assumed, and this is the one place I
   deliberately did not claim a verdict.** Reading the adapter
   (`python/smelt/bigquery_adapter.py:88-94` → `result.to_arrow()` → `to_batches()` →
   `RecordBatch::from_pyarrow_bound`), google-cloud-bigquery maps a `REPEATED STRING` field to
   `list<…: string>`, which the existing `downcast_ref::<ListArray>()`/`StringArray` path
   handles. I cannot *prove* that offline — the storage-API path and the client version both
   sit outside this repo. So rather than assert it, `decode_string_list_column`
   (`maintenance_driver/observed_delta.rs:76`) was rewritten to remove the failure mode
   entirely: it accepts `ListArray`/`LargeListArray` over `StringArray`/`LargeStringArray`, and
   **errors** on any other shape or a missing column instead of returning an empty vector. The
   old silent empty was harmless with one in-process producer and is a silent-narrowing hazard
   with an adapter: a consumer cannot tell an empty decode from an empty delta, so an
   unrecognised shape would restrict a recompute to *no* keys instead of widening. Four unit
   tests, including the refusals.

## What else landed

- **`ddl_bigquery.rs` became a directory** (`ddl_bigquery/{mod,ledger,observed_delta}.rs`) —
  the file was at its 1007-line baseline and this phase adds ~300. Split, not raised: the only
  baseline change is `--update` dropping the now-orphaned `ddl_bigquery.rs` row, and no new file
  is near the cap. `escape_string_literal` and `qualified` moved to `mod.rs` so both state
  modules share one escaper. Public paths are unchanged (`pub use`), so no caller moved.
- **One dispatch point, matching phase 13's** — `crates/smelt-state/src/observed_delta.rs`
  (new): three functions keyed on `SqlDialect`, exhaustive, Spark an error naming the dialect.
  All four run-layer sites route through it (`driver.rs:743,746`, `column_scoped.rs:388,403`,
  `membership/execute.rs:92,105`, `observed_delta.rs:175,179`); the three
  `use smelt_state::ddl_duckdb` imports under `maintenance_driver/` are gone.
- **The row flipped.** `realisable_state_structures(BigQuery)` is now
  `vec![MergeLedger, ReconciliationLedger, ObservedOutputDeltas]`, rebased on phases 13/14.

## Decisions that diverge from, or go beyond, the plan

1. **`ObservedDelta` stayed in `ddl_duckdb`.** The plan called moving it optional. It is the
   decoded *shape* of a row, identical in both dialects, and it has callers across four crates;
   moving it would be a rename touching files this phase has no other business in. Noted as a
   misnomer, left for a phase already in those files.
2. **Plan test 5 ("BigQuery records no `ObservedOutputDeltas` downgrade") is vacuous as
   written, and was replaced.** No technique maps to `ObservedOutputDeltas` in
   `required_state_structure` — it is precision-only, "simply not recorded" per the degradation
   contract — so `resolve_availability` never records a downgrade for it on any dialect. The
   meaningful assertion is at the run layer, and it is now
   `observed_delta::degradation::records_observed_deltas_follows_the_availability_row`,
   exhaustive over the dialects.
3. **`FingerprintSidecar` stayed off**, as scoped. The phase-11 contradiction test
   (`the_sidecar_claim_matches_the_backend_capability`) is intact and still green.

## Tests

| Test | What it holds |
|---|---|
| `smelt-state` lib, `ddl_bigquery::observed_delta::tests` (6) | Verbatim GoogleSQL for all three builders; `IGNORE NULLS` + `CAST` on both aggregates; no `ON CONFLICT`/`FILTER (WHERE`/`::VARCHAR[]`/`"`; the present-and-empty shape; backslash escaping; project-prefixed schema. |
| `smelt-state --test ledger_dialect` (+5) | Dispatch exhaustive and Spark refused by name; no DuckDB spelling on the BigQuery path; DuckDB path byte-identical to its builders; non-vacuity; and the empty-vs-absent row-presence test. |
| `smelt-backend-bigquery` `sql::tests` (+3) | §0: the creating-write degradation and its reported verdict, the DML non-vacuity, the `CREATE`/`TEMP`/DML classifier. |
| `smelt-runtime --test observed_delta` (+2) | The BigQuery realisation leg (GoogleSQL record actually emitted through the real driver + seam), and the per-dialect predicate. |
| `smelt-runtime` lib, `observed_delta::decode_tests` (4) | List/large-list × string/large-string decode; refusal on an unrecognised shape, a non-string element type, and a missing column; NULL list entry → empty. |

**On red-green.** The §0 tests could not have compiled against the old signature, let alone
passed — the old function returned `Vec<String>` and bound the `CREATE` into the transaction.
The BigQuery realisation test fails on the pre-flip row with the record absent from the call
log. The decode refusals fail on the old body by returning `Ok(vec![])`. Each red was
established by the change itself rather than by a separately staged run.

## Pre-existing tests corrected (never deleted)

| Test | Why it moved |
|---|---|
| `maintenance_availability/realisation.rs::has_emitters` | BigQuery's `ObservedOutputDeltas` cell is now backed. |
| `…::each_dialect_realises_exactly_the_structures_it_has_today` | BigQuery is `{MergeLedger, ReconciliationLedger, ObservedOutputDeltas}`; the message now names the two rows that remain. |
| `maintenance_availability/succession.rs::a_ledger_less_dialect_realises_no_ledger` | The observed-delta assertion is Spark-only now; the loop's remaining both-dialect claim is the fingerprint sidecar. |
| `observed_delta/degradation.rs::KeyedNonDuckDbBackend` | Gained a `dialect` field (and dialect-matched capabilities) so the same fake serves both legs; the Spark test itself is unchanged in substance. |
| `sql::tests` bookkeeping trio | Read `plan.statements` and assert the new `atomicity` verdict. |

## Gates

| Gate | Result |
|---|---|
| `bash .claude/scripts/verify-phase.sh` | ALL GREEN |
| `cargo test -p smelt-runtime --test observed_delta` | 16 passed |
| `cargo test -p smelt-runtime --test state_guard_census` | 3 passed |
| `cargo test -p smelt-runtime --test availability_seam` | 6 passed |
| `cargo test -p smelt-backend-bigquery --test never_fold_twice` | 3 passed (the gate list says `-p smelt-runtime`; the target lives in `smelt-backend-bigquery`, where phase 14 put it) |
| `cargo test -p smelt-logical --test maintenance_availability` | 22 passed |
| `cargo test -p smelt-logical --test maintenance_dialect_blindness` | 3 passed |
| `cargo test -p smelt-state --test ledger_dialect` | 12 passed |
| `cargo check -p smelt-cli --features bigquery` | clean |
| `bash .claude/scripts/large-file-check.sh` | OK — one orphaned row dropped by `--update`, none raised |

No live warehouse was reached.

## Spec delta

`docs/specs/state.md` §"Which dialects realise which structure": the observed-delta row is
`yes` for BigQuery; a new three-item list states the `MERGE` upsert, the
`ARRAY_AGG(DISTINCT CAST(… AS STRING) IGNORE NULLS)` expression with each of its three reasons,
and the row-presence basis of empty-vs-absent (with BigQuery's array flattening named as the
reason it needs saying); and a new paragraph records the one shape BigQuery **degrades** rather
than binds atomically (§0), distinct from the one it refuses. Spark's row is untouched.

## What phases 15 and 16 inherit

- **Row 15** inherits the same two patterns now proven twice each: a `SqlDialect`-keyed
  dispatch module in `smelt-state` per structure, and a run-layer predicate derived from
  `realisable_state_structures`. It also inherits §0's verdict in its own form — the tombstone
  patch's transaction must not contain permanent-entity DDL either, and `BookkeepingAtomicity`
  is now the vocabulary for saying so when it does.
- **Row 16** inherits phases 13/14's four live checks, plus two from here:
  1. **The Arrow list shape BigQuery's adapter actually returns for `ARRAY<STRING>`** — the
     decode now refuses rather than silently emptying on anything unexpected, so a live run
     either decodes or says exactly what it got. This is the check that could not be settled
     offline, and it is now designed to fail loudly rather than quietly.
  2. **That the `MERGE` upsert, the `IGNORE NULLS` aggregates and the `CAST` all run** against
     a real warehouse — in particular that `ARRAY<STRING>` columns declared without `NOT NULL`
     accept the write, and that a fully-suppressed window really does land one present-and-empty
     row.
