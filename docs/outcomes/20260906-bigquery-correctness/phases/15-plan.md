# Phase 15 — The tombstone ledger on BigQuery

**Outcome:** `docs/outcomes/20260906-bigquery-correctness/outcome.md`
**Row:** 15 — "Tombstone ledger on BigQuery, so the succession-patch technique runs live rather
than downgrading to `DeleteInsert`"
**Criteria served:** 8, 9, 3, 7
**Spec anchors:** `docs/specs/incremental_shapes.md` §"The tombstone ledger (hidden state)"
(Physical shape, Lifecycle); `docs/specs/state.md` §"Which dialects realise which structure",
§"The degradation contract"; `docs/specs/incremental_models.md` §"Statement emission (single
owner)"

This is the last of the four state structures and **the one that is unlike the other three.**
Read §"Why this phase is shaped differently" before planning; the pattern phases 13 and 14
established does not transfer wholesale.

## Where phases 11–14 left it

- `realisable_state_structures(BigQuery)` holds `MergeLedger`, `ReconciliationLedger` and
  (after phase 12) `ObservedOutputDeltas`. **Rebase on whatever is actually there; do not
  restate it from this plan.**
- `TombstoneLedger` is now the **only** raw `STATE-GUARD` left —
  `crates/smelt-runtime/src/maintenance_driver/succession/execute.rs:98` and `:313` (line
  numbers may have shifted; find them by the marker).
- Established patterns to follow, not reinvent: the exhaustive `SqlDialect` dispatch in
  `crates/smelt-state/src/ledger.rs`; `escape_string_literal`; `ddl_bigquery::bigquery_type_sql`
  for per-type SQL; derived predicates in `maintenance_driver/ledger.rs`
  (`realises_merge_ledger`, `realises_reconciliation_ledger`) in place of `!= DuckDB`
  comparisons; pure-plan-builder tests; `supports_transactional_ddl` now being honest for
  BigQuery.

## Why this phase is shaped differently

The merge, reconciliation and observed-delta ledgers are **bookkeeping** — `CLAUDE.md`
§"Maintenance-plan purity" explicitly excludes their DDL/DML in `smelt-state`, which is why
phases 12–14 could write GoogleSQL directly there.

The tombstone ledger is not only bookkeeping. Its **statements are maintenance statements**, and
maintenance-plan purity binds them: "every maintenance statement a run executes is the output of
a pure emitter in `smelt-logical`'s maintenance layer — backends execute, never author". So the
work splits across two layers with different rules:

- `crates/smelt-state/src/ddl_duckdb.rs:916` `generate_tombstone_table_ddl` /
  `generate_tombstone_table_drop_ddl` — the ledger *table* DDL. Bookkeeping; belongs in
  `smelt-state`, gets a BigQuery sibling and joins the dispatch. Note it currently renders
  column types with `DataType::to_sql()`, which is DuckDB's spelling — BigQuery needs
  `ddl_bigquery::bigquery_type_sql`, and that function returns `Result`, so the DDL builder's
  signature has to grow a failure path rather than swallowing an unmappable type.
- `crates/smelt-logical/src/maintenance/emit/succession.rs` — the patch, rebuild, event-delta,
  ledger-rebuild-select and clock-tie-probe statements. These stay single-owned in
  `smelt-logical` and must become dialect-plural **there**. Do not move them, do not let the
  backend author a variant, and do not let `smelt-runtime` patch the text after emission —
  `cargo test -p smelt-runtime --test statement_parity` gates exactly that.

**The two emitters that block everything** are `emit_succession_patch` (line ~184) and
`emit_succession_full_rebuild` (line ~367). Both open with
`assert!(matches!(dialect, MaintenanceDialect::DuckDb), "…only MaintenanceDialect::DuckDb is
supported today…")`. Two consequences:

1. Making them emit GoogleSQL **is** the substance of this phase. Budget accordingly — this is
   larger than 13 or 14, and it is where the real SQL translation risk lives.
2. Those `assert!`s are panics on a dialect input, which is a fail-loud violation in spirit
   (`CLAUDE.md` §"Fail-loud discipline" wants a diagnostic, not an abort). If a dialect remains
   unsupported after this phase, convert the panic to a typed refusal on the way past; say so
   either way.

## Work

### 1. Read the statements before translating them

`emit_succession_patch` builds a `StatementGroup` including an anti-joined idempotent
`INSERT INTO <presented>__tombstones …`; `emit_succession_full_rebuild` folds on
`(key_cols, clock_col)` with a per-column aggregate (phase 3 rewrote it) and runs the clock-tie
probe. `emit_succession_union_relation` unions presented rows, ledger rows (NULL payload) and
the batch. Enumerate every construct each emits and check each against GoogleSQL before writing
anything — this phase's failure mode is a plausible-looking translation that the engine rejects
on a path no offline test covers. Constructs to be suspicious of, non-exhaustively: `USING`
joins, `EXCEPT`/anti-join spellings, `QUALIFY`, window frames, `IS NOT DISTINCT FROM` (GoogleSQL
has no such operator — the codebase's null-safe join convention is already recorded in
`docs/specs/multi_backend.md` §"Statement-level lowering"), and any `VARCHAR`/`::` cast.

Prefer reusing the registry and the existing dialect seams over hand-spelling: if a difference
is a *function spelling*, it is `Signature::emission` data
(`crates/smelt-types/src/signatures/`); if it is a *capability*, it is a `BackendCapabilities`
flag. A hand-written `match dialect` in the printer's territory will fail
`cargo test -p smelt-dialect --test emission_ownership`.

### 2. `DataType` → GoogleSQL in the ledger DDL

The tombstone table's columns are the model's own inferred types. Route through
`bigquery_type_sql` and propagate its `Result`. A type with no BigQuery mapping must produce a
diagnostic naming the column and the type, never a silent substitution or an `Unknown`
(`CLAUDE.md` §"Fail-loud discipline"; the `error`-Unknown guard).

### 3. Dispatch, and the two guards

Add the tombstone DDL to the same dialect dispatch, Spark erroring by name. Then retire the two
`STATE-GUARD` bails in favour of a derived `realises_tombstone_ledger` predicate in
`maintenance_driver/ledger.rs`, following phase 14's precedent — the census module doc names the
derived predicate as the preferred fix and phases 13/14 both took it. After this, the census's
guard list should be **empty**; make sure the census still fails closed on an empty list rather
than passing vacuously. (Phase 8 had to prove exactly this for the divergence registry — same
question, same answer required.)

### 4. Flip the row, and check what stops downgrading

`realisable_state_structures(SqlDialect::BigQuery)` gains `TombstoneLedger`. A `TombstonePatch`
cell with no realisable ledger currently downgrades to `DeleteInsert` (full refresh); on
BigQuery it should now run the patch. That change is the row's stated point ("so the
succession-patch technique runs live rather than downgrading"), so **find the cell it affects
and assert the absence of the downgrade**, the way phases 13 and 14 each did in
`example_diagnostics`. If no model in `examples/` reaches it, say so plainly — that is a real
finding about criterion 9's reach, not a gap to paper over.

Expect fallout in tests encoding "BigQuery realises nothing here", including
`crates/smelt-logical/tests/maintenance_availability/succession.rs`. **Correct them; never
delete them.**

### 5. If it cannot be finished honestly, stop at the honest line

If a construct in the patch or rebuild emitters has no sound GoogleSQL realisation, the correct
outcome is **not** a partial flip. Leave `TombstoneLedger` off for BigQuery, keep the refusal
(as a typed refusal, per §"Why this phase is shaped differently" point 2), record the reason in
`docs/specs/state.md` the way Spark's permanent absence is recorded, and say exactly which
construct blocked it. The outcome's own framing supports this — Spark's realisation was refused
rather than deferred, with the reason written into the spec. A false realisation is the one
outcome that is worse than an unfinished row.

## Tests (red-green, written before the fix)

1. **Emitter unit tests per dialect** for `emit_succession_patch` and
   `emit_succession_full_rebuild` — the BigQuery text asserted verbatim, with the DuckDB text
   unchanged (a regression there is the likeliest accident).
2. **Tombstone DDL** in GoogleSQL: `NOT NULL` columns in BigQuery types, `PRIMARY KEY (k…, t)
   NOT ENFORCED`, backticked name; plus the unmappable-type refusal.
3. **Dialect blindness:** extend `cargo test -p smelt-logical --test
   maintenance_dialect_blindness` — its whole subject is an emitter taking a `dialect` and
   hardcoding DuckDB anyway, which is precisely the risk here.
4. **Statement parity:** `cargo test -p smelt-runtime --test statement_parity` must stay green,
   including its structural no-authoring leg — the proof that the new GoogleSQL came from
   `smelt-logical` and not from the backend or the driver.
5. **Census:** green with both `TombstoneLedger` guards retired, and non-vacuous on an empty
   guard list.
6. **Availability:** BigQuery records no `TombstoneLedger` downgrade; Spark still does.
7. The §4 assertion that a real cell stopped being coarsened (or the finding that none exists).

**Do not reach a live BigQuery warehouse.** Offline proof plus `cargo check -p smelt-cli
--features bigquery`. Phase 16 owns live verification and already inherits a list; add to it.

## Spec delta (spec-first)

- `docs/specs/state.md` §"Which dialects realise which structure": BigQuery's tombstone-ledger
  row moves to realised (or stays, with the blocking construct named, per §5).
- `docs/specs/incremental_shapes.md` §"The tombstone ledger (hidden state)": "Physical shape"
  and "Lifecycle" are written against one dialect's spelling; make them dialect-plural the way
  the reopening did for never-fold-twice.

## Gates

```
bash .claude/scripts/verify-phase.sh
cargo test -p smelt-runtime --test statement_parity
cargo test -p smelt-runtime --test state_guard_census
cargo test -p smelt-runtime --test availability_seam
cargo test -p smelt-runtime --test observed_delta
cargo test -p smelt-backend-bigquery --test never_fold_twice
cargo test -p smelt-logical --test maintenance_availability
cargo test -p smelt-logical --test maintenance_dialect_blindness
cargo test -p smelt-dialect --test emission_ownership
cargo test -p smelt-state --test ledger_dialect
cargo check -p smelt-cli --features bigquery
bash .claude/scripts/large-file-check.sh
```

Check the large-file baseline before and after; `succession.rs` and `ddl_bigquery.rs` are both
candidates to tip over. Split (directory form, as phase 14 did for `maintenance_driver/tests.rs`
— `statement_parity`'s no-authoring gate requires that shape) rather than raising a ratchet.

## Deliverables

- The code + tests above.
- `docs/outcomes/20260906-bigquery-correctness/phases/15-summary.md`.
- A decision-log entry at the **top** of the outcome's `## Decision log`, in the voice of the
  existing entries: dense, file:line specific, explicit about what is not proven offline.
- Row 15 flipped to `done` (or left `pending` with §5's honest refusal recorded — say which and
  why).
- One commit on `bigquery-prod` with the session's trailers.
