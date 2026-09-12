# Phase 13 — Merge ledger and reconciliation ledger on BigQuery

**Outcome:** `docs/outcomes/20260906-bigquery-correctness/outcome.md`
**Row:** 13 — "Merge ledger and reconciliation ledger on BigQuery: `ON CONFLICT DO NOTHING`
re-expressed as `MERGE … WHEN NOT MATCHED`, plus the `execute_write_with_bookkeeping` override"
**Criteria served:** 8 (plan layer and run layer never disagree about state), 9 (BigQuery runs
the pipeline's real plan), 3 (every fix is gated), 7 (gates green)
**Spec anchors:** `docs/specs/state.md` §"The state-structure inventory", §"Which dialects
realise which structure", §"The degradation contract"; `docs/specs/incremental_shapes.md`
§"The transactional frontier write (merge ledger)"; `docs/specs/incremental_models.md`
§"The frontier record (reconciliation ledger)", §"Statement emission (single owner)"

## What phase 11 left for this phase

Phase 11 made `realisable_state_structures(BigQuery)` return `vec![]` — an honest empty row —
and put the structural census (`cargo test -p smelt-runtime --test state_guard_census`) behind
it: every `SqlDialect::DuckDB` comparison under `crates/smelt-runtime/src/maintenance_driver/`
carries `// STATE-GUARD: <StateStructure>` and that structure must be unrealisable off DuckDB.
Four guards remain:

| site | structure |
|---|---|
| `maintenance_driver/driver.rs:611` | `MergeLedger` |
| `maintenance_driver/driver.rs:485` | `ReconciliationLedger` |
| `maintenance_driver/succession/execute.rs:97` | `TombstoneLedger` |
| `maintenance_driver/succession/execute.rs:312` | `TombstoneLedger` |

The census is the ratchet: flipping a row on without deleting its guard fails, and deleting a
guard without backing it fails. This phase owns the first row; phase 15 owns the last two.

## The scope decision this phase must make first (and the plan's answer)

`ReconciliationLedger` is **not** one leg. `technique_state_structure` maps
`Technique::KeyedFold → ReconciliationLedger`, and the guard at `driver.rs:485` sits inside the
`Grade::Additive` arm — the never-fold-twice refusal, which today is a DuckDB `PRIMARY KEY`
constraint violation caught as `BackendError::already_reflected`
(`smelt-backend-duckdb/src/lib.rs:748-755`). BigQuery's `PRIMARY KEY`s are declared and
**unenforced**, so that refusal has to be re-expressed transactionally — which is row 14's
entire subject, and the outcome's reopening entry calls it "the load-bearing phase: get it
wrong and an additive fold double-counts".

So this phase **flips `MergeLedger` on for BigQuery and leaves `ReconciliationLedger` off**,
while building every piece of reconciliation-ledger *SQL and seam* that phase 14 then only has
to make refuse correctly. Concretely:

- `driver.rs:611`'s `MergeLedger` guard is deleted (the census then forces it).
- `driver.rs:485`'s `ReconciliationLedger` guard stays, annotation intact, and its comment is
  updated to say the ledger SQL now exists on BigQuery and what is missing is the enforced
  never-fold-twice refusal, naming row 14.
- If the implementer finds this reading wrong — e.g. the `Additive` arm turns out to be
  unreachable for BigQuery for an independent reason, or `fold_ledger_delta` can be made sound
  on BigQuery inside this phase without the transactional refusal work — say so in the summary
  and adjust, rather than silently flipping both rows.

## Work

### 1. BigQuery ledger SQL (`crates/smelt-state/src/ddl_bigquery.rs`)

Port every ledger builder `ddl_duckdb.rs` holds, in GoogleSQL:

- `generate_ledger_table_ddl` — `CREATE TABLE IF NOT EXISTS`, `STRING NOT NULL` columns,
  `PRIMARY KEY (…) NOT ENFORCED` (GoogleSQL requires the `NOT ENFORCED` suffix; a bare
  `PRIMARY KEY` is a syntax error). The declaration is documentation and an optimiser hint
  here, never a constraint — say so in the doc comment, because phase 14 depends on the
  distinction.
- `generate_ledger_insert_sql` — same shape.
- `generate_ledger_upsert_sql` — **this is the row's named defect.** GoogleSQL has no
  `ON CONFLICT DO NOTHING`. Re-express as
  `MERGE <ledger> T USING (SELECT … ) S ON T.model_name = S.model_name AND … WHEN NOT MATCHED
  THEN INSERT …`. Use `SELECT <literals>` (or `UNNEST([STRUCT(…)])`) for the single-row source;
  note that phase 12's finding 1b established `UNNEST([STRUCT(…)])` as the scaling form for
  *many* rows — one row does not need it, so pick the simpler spelling and say why.
- `generate_ledger_exists_sql`, `generate_ledger_recompute_reset_sqls` — same shapes.

Identifier quoting is the trap: `ddl_duckdb::quote_identifier` emits `"schema"`, which GoogleSQL
rejects. Reuse the backtick conventions already in `crates/smelt-backend-bigquery/src/sql.rs`
(`qualified_name`) rather than inventing a second quoting rule; the driver passes only `schema`,
so a two-part `` `schema`.`_smelt_ledger` `` resolving against the job's default project is the
expected form — confirm against `sql.rs` and state the choice in the doc comment.

Keep the builders in `smelt-state` (`CLAUDE.md` §"Maintenance-plan purity" excludes ledger
DDL/DML there as bookkeeping — do **not** move them into `smelt-logical`'s emit layer).

### 2. Dialect dispatch at the call sites

`maintenance_driver/driver.rs` calls `ddl_duckdb::generate_ledger_*` by name. Do not scatter
`match dialect` at each call site: add one dispatch point (a small function or module in
`smelt-state`, keyed on `SqlDialect`, exhaustive so a new dialect is a compile error) and route
the driver through it. Spark must **refuse or stay unrealised** — phase 11 already recorded why
(no cross-table transaction in Delta); do not give Spark a ledger spelling here.

### 3. `execute_write_with_bookkeeping` override on BigQuery

`crates/smelt-backend-bigquery/src/lib.rs` today overrides only
`delete_and_insert_transactional`, so bookkeeping and write are sequential and non-atomic.
Override `execute_write_with_bookkeeping` to run `pre_write_sqls` + `write_group` inside one
`BEGIN TRANSACTION … COMMIT` multi-statement query job, with `ensure_sqls` executed first and
**outside** it (DDL is not permitted inside a BigQuery transaction — this matches the trait's
own documented precedent for `ensure_sql`). Roll back on failure (`ROLLBACK TRANSACTION`, or
rely on the job's own abort — whichever the adapter actually gives; verify and state which).
This is the seam `execute_conditional_write_and_record_observed_delta` delegates to, so the
override is shared with row 12's work — if row 12 is still `pending` when this lands, note in
the summary that it is now unblocked on this half.

### 4. Flip the row and let the census bite

`realisable_state_structures(SqlDialect::BigQuery)` → `vec![StateStructure::MergeLedger]`.
Update the function's doc comment (it currently says "Today only DuckDB has any builder" —
that becomes false). Delete `driver.rs:611`'s guard and its now-stale "the ledger substrate is
DuckDB-only today" prose.

Expect pre-existing tests that encode the old claim to fail — phase 11 hit exactly this and the
rule is **correct them, do not delete them**. Likely candidates:
`crates/smelt-logical/tests/maintenance_availability/{resolution,succession,realisation}.rs`,
`crates/smelt-runtime/tests/availability_seam*`, `maintenance_dialect_blindness`.

## Tests (red-green, written before the fix)

1. **Unit, `smelt-state`:** each new BigQuery ledger builder's emitted text, asserted verbatim —
   especially that the upsert is a `MERGE … WHEN NOT MATCHED` and contains no
   `ON CONFLICT`, and that the DDL says `NOT ENFORCED`.
2. **Dialect-blindness:** a test that the dispatch is exhaustive and that the BigQuery path
   never returns DuckDB-flavoured text (no `"`-quoted identifier, no `ON CONFLICT`). If
   `cargo test -p smelt-logical --test maintenance_dialect_blindness` can carry it, put it
   there; otherwise a sibling scan in `smelt-state`.
3. **Backend seam:** a test that BigQuery's `execute_write_with_bookkeeping` puts
   `pre_write_sqls` + the write group in one transaction and `ensure_sqls` outside it —
   asserted against a recorded statement log, not a live warehouse.
4. **Census:** `cargo test -p smelt-runtime --test state_guard_census` green with the
   `MergeLedger` guard gone and the `ReconciliationLedger` guard still annotated.
5. **Availability:** BigQuery resolving a `Grade::Idempotent` windowed-keyed cell records **no**
   `MergeLedger` downgrade, and still records the `ReconciliationLedger` one for `KeyedFold`.

Every one of these is provable offline. **Do not reach a live warehouse** — `cargo check -p
smelt-cli --features bigquery` is the compile-side proof, per the outcome's phase preamble.

## Spec delta (spec-first)

`docs/specs/state.md` §"Which dialects realise which structure": BigQuery's merge-ledger row
moves from "not yet" to realised, with the two-part name and the `NOT ENFORCED` primary key
stated as facts of the realisation, and the reconciliation-ledger row keeps its "not yet"
with the unenforced-PK reason named (that is what row 14 removes). Spark's rows are untouched.
If the `MERGE`-as-upsert re-expression changes any user-visible statement text, it belongs in
`docs/specs/incremental_shapes.md` §"The transactional frontier write (merge ledger)" too.

## Gates

```
bash .claude/scripts/verify-phase.sh
cargo test -p smelt-runtime --test state_guard_census
cargo test -p smelt-runtime --test availability_seam
cargo test -p smelt-logical --test maintenance_availability
cargo test -p smelt-logical --test maintenance_dialect_blindness
cargo check -p smelt-cli --features bigquery
bash .claude/scripts/large-file-check.sh
```

`ddl_bigquery.rs` is 607 lines and `ddl_duckdb.rs` 1883 — check the large-file baseline before
and after; if the additions push `ddl_bigquery.rs` over its cap, split by structure
(`ddl_bigquery/{mod.rs,ledger.rs}`) rather than raising the ratchet.

## Deliverables

- The code + tests above.
- `docs/outcomes/20260906-bigquery-correctness/phases/13-summary.md`.
- A decision-log entry appended to the outcome's `## Decision log` (newest first), and row 13
  flipped to `done` in the phase table.
- One commit, message prefix `feat(state):` or `outcome(bigquery-correctness):`, ending with the
  session's Co-Authored-By / Claude-Session trailers.
