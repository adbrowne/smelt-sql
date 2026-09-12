# Phase 14 summary — the never-fold-twice refusal is real on BigQuery

**Row:** 14 · **Criteria served:** 8, 9, 3, 7 · **Status:** done

## What landed

**The refusal moved from the storage engine into the statement.** On DuckDB the
never-fold-twice guarantee *is* the ledger table's enforced `PRIMARY KEY`: a repeat insert
violates it, `is_constraint_violation` recognises the violation, and the `duckdb::Transaction`
rolls back before `action_sql` runs. BigQuery's key is `NOT ENFORCED` and raises nothing. The
refusal is now an *effect* test instead, and it lives in three pieces:

- `smelt_state::ledger::ledger_fold_record_sql` (new dispatch function) — the record whose
  zero-effect outcome is the refusal. DuckDB: a plain `INSERT` against the enforced key.
  BigQuery: `ddl_bigquery::generate_ledger_conditional_insert_sql`, which delegates verbatim
  to the phase-13 `MERGE … WHEN NOT MATCHED THEN INSERT` — one builder, two roles, so the
  additive fold and the idempotent bookkeeping record can never disagree about what "this
  window is recorded" means.
- `smelt_backend_bigquery::sql::fold_ledger_delta_script` — the GoogleSQL script:
  `BEGIN / BEGIN TRANSACTION; <record>; IF @@row_count = 0 THEN RAISE USING MESSAGE =
  '<sentinel>: …'; END IF; <action>; COMMIT TRANSACTION; EXCEPTION WHEN ERROR THEN ROLLBACK
  TRANSACTION; RAISE USING MESSAGE = @@error.message; END;`. The `RAISE` inside the `BEGIN`
  section is caught by the handler, which rolls back and re-raises carrying the sentinel.
- `BigQueryBackend::fold_ledger_delta` — the override. The trait default (a documented
  best-effort `exists` → `insert` → `action` across three jobs) is **not** used: its
  check-then-act window is the exact defect the row exists to prevent.

**The row flipped, and the guard is gone rather than annotated.**
`realisable_state_structures(BigQuery)` is now `vec![MergeLedger, ReconciliationLedger]`.
`driver.rs`'s `dialect != SqlDialect::DuckDB` guard was replaced by
`maintenance_driver::realises_reconciliation_ledger`, derived from the availability layer —
the same treatment phase 13 gave `MergeLedger`. `state_guard_census` now covers two guards
(both `TombstoneLedger`, phase 15's), and `ReconciliationLedger` joins `ObservedOutputDeltas`
and `MergeLedger` in the list of structures whose gate **must** be derived, never compared.

**The legible result, again from a real workspace.** `examples/github_activity`'s
`KeyedFold` cell on `raw.github_events` no longer emits `MaintenanceStateDowngraded` on the
`bigquery` target. Its absence is now the asserted claim
(`example_diagnostics/smoke_and_migration.rs`), and the fixture is down to two diagnostics,
both `SuccessionPatch`/tombstone-ledger — phase 15's.

**A capability was wrong and is now right.** `BackendCapabilities::bigquery()` declared
`supports_transactional_ddl: true`. BigQuery's docs are explicit that DDL creating or
dropping *permanent* entities is not supported inside a transaction (only `CREATE TEMP TABLE`
and friends), and every smelt caller of the flag asks about a permanent table. It is now
`false`, with the quote in the comment and the `capability_conformance` cell corrected. This
changes no behaviour today — BigQuery does not override `execute_statement_group`, whose
default ignores `StatementGroup::transactional` — but it is the flag the new driver refusal
keys on.

## The plan's four explicit decisions

1. **The sentinel.** `SMELT_LEDGER_ALREADY_REFLECTED`, declared as
   `sql::ALREADY_REFLECTED_SENTINEL` and used by *both* the emitter
   (`fold_ledger_delta_script`) and the matcher (`is_already_reflected`) in the same module,
   so they cannot drift — `sentinel_the_script_emits_is_the_sentinel_the_matcher_knows`
   asserts the pairing against a message shaped like the adapter's real job-error envelope.
   The matcher is a `contains`, not an equality, for exactly that reason. Screaming snake
   case with a `SMELT_` prefix so it cannot collide with user data reaching an error string.
2. **Isolation — checked against the docs, and the argument is stronger than snapshot
   isolation.** BigQuery's "Multi-statement transactions" page states two things: transactions
   "guarantee ACID properties and support snapshot isolation", and — the load-bearing sentence
   — "If a transaction mutates (updates or deletes) rows in a table, then other transactions
   or DML statements that mutate rows in the same table cannot run concurrently. Conflicting
   transactions are cancelled." Both folds of one delta mutate `_smelt_ledger`, so they cannot
   both commit: one wins, the other is cancelled loudly, and a later re-run reads the committed
   row and refuses. The refusal therefore rests on *write-conflict detection on one table*, not
   on read-snapshot reasoning, which is a stronger and more directly applicable guarantee than
   the plan anticipated. The argument is written into
   `fold_ledger_delta_script`'s doc comment and into `docs/specs/state.md`.
3. **DDL in the transaction — option (a), refuse, because (b) is provably false.** The plan
   offered "refuse the DDL-shaped action" or "prove the additive path never produces DDL".
   The call site settles it against (b): `driver.rs`'s `action_group` is `create_group`
   whenever `table_exists` is false, and `create_group` is `emit_create_table_as`. So the
   *first* step of an additive fold against a fresh target genuinely is DDL. It is refused,
   before any backend call, with the construct and the remedy named — `--full-refresh`
   materialises the target outside the window-forward loop, after which every step is a merge
   the transaction can hold. The condition is spelled as the capability it is
   (`!backend.capabilities().supports_transactional_ddl`), never as a dialect comparison, so
   the census does not have to be told about it and DuckDB — whose transactions roll a
   `CREATE TABLE` back for free — is unaffected. Both halves are tested: the refusal, and
   (non-vacuity) that the same BigQuery cell folds normally once the target exists.
4. **`exists_sql` is dead on BigQuery, and says so.** The override binds it as `_exists_sql`
   with a doc paragraph explaining that a separate existence probe is precisely the
   check-then-act window the override exists to close, so issuing it would cost a job and buy
   nothing. It stays in the trait signature because the DuckDB-era default still uses it.

## Divergences from the plan

- **The conditional insert is the phase-13 `MERGE`, not a new `INSERT … WHERE NOT EXISTS`.**
  The plan sketched `INSERT … SELECT <literals> WHERE NOT EXISTS (…)`. That shape needs a
  dummy `FROM` in GoogleSQL (a bare `SELECT … WHERE` has no table to filter) and would have
  been a second, untested spelling of "record if absent". Reusing
  `generate_ledger_upsert_sql` gives the identical zero-rows-on-repeat effect with text that
  already has verbatim tests, and keeps the additive and idempotent records definitionally
  identical.
- **A file split, not a ratchet raise.** `ddl_bigquery.rs` grew only ~40 lines (967 → 1007)
  and needed no split; the file that blew its budget was
  `maintenance_driver/tests.rs` (1083 → 1205). It became `maintenance_driver/tests/mod.rs`
  plus `tests/ledger.rs`. The directory form is deliberate: `statement_parity`'s
  no-authoring gate skips directories named `tests`, and a flat `ledger_tests.rs` tripped it
  on `SumRuleAdditive`'s fixture `MERGE INTO` text. No baseline entry was raised;
  `--update` dropped the stale `tests.rs` row and added `ddl_bigquery.rs 1007`.

## Tests

| Test | What it holds |
|---|---|
| `smelt-backend-bigquery --test never_fold_twice` (3, new) | **The headline.** Drives the real `ledger_fold_record_sql` + `fold_ledger_delta_script` through a GoogleSQL script executor that models only the three engine behaviours the refusal needs, and reads the script text rather than being told its shape. The same delta folded twice: second refused with the sentinel, and the action absent from the statement log. Plus: a *different* delta folds normally (non-vacuity), and a *fresh* run against an already-recorded delta refuses (the re-run case). |
| `smelt-backend-bigquery` `sql::tests` (+3) | Sentinel emitter/matcher pairing; record → abort → action inside exactly one `BEGIN TRANSACTION … COMMIT TRANSACTION` with an explicit `ROLLBACK` handler; `;`-termination exactly once. |
| `smelt-state --test ledger_dialect` (+2, and `all_statements` widened) | The fold record refuses a repeat in each dialect's own way (DuckDB `INSERT`, BigQuery `MERGE … WHEN NOT MATCHED` with no matched arm, Spark refused); no `''` quote-doubling anywhere on the BigQuery path. |
| `maintenance_driver::tests::ledger` (+2) | The first-run DDL refusal names `CREATE TABLE` and `--full-refresh` and issues no SQL; and the same cell folds through GoogleSQL ledger statements once the target exists. |
| `maintenance_availability::realisation` | `bigquery_keeps_both_a_merge_ledger_and_a_keyed_fold_technique` — the **absence** of a downgrade is the claim; `a_dialect_without_the_reconciliation_ledger_still_downgrades_a_keyed_fold` keeps it non-vacuous on Spark. |

**The red was verified to be the right red.** Pointing BigQuery's fold record back at
`generate_ledger_insert_sql` makes the headline test fail with "a repeat fold must be refused,
got Committed" — the double-count itself — not with an incidental parse error. The fake
warehouse models an unenforced key honestly (a duplicate `INSERT` simply lands again), which
is what makes that red meaningful.

## Pre-existing tests corrected (never deleted)

| Test | Why it moved |
|---|---|
| `realisation.rs::has_emitters` | BigQuery's `ReconciliationLedger` cell is now backed. |
| `realisation.rs::each_dialect_realises_exactly_the_structures_it_has_today` | BigQuery is `{MergeLedger, ReconciliationLedger}`. |
| `realisation.rs::bigquery_keeps_a_merge_ledger_technique_and_still_downgrades_a_keyed_fold` | Renamed and inverted; the Spark half split into its own non-vacuity test. |
| `succession.rs::a_ledger_less_dialect_realises_no_ledger` | The reconciliation-ledger assertion is Spark-only now. |
| `state_guard_census.rs::the_census_is_non_empty_and_covers_the_known_guards` | Three known guards → two; `ReconciliationLedger` joins the derive-don't-compare list. |
| `maintenance_driver` `additive_grade_on_non_duckdb_backend_fails_loud` | Renamed `…_on_a_dialect_with_no_refusal_fails_loud` — "non-DuckDB" stopped being the condition. |
| `RecordingBackend::capabilities` | Was pinned to `duckdb()` regardless of dialect, which would have made the new refusal untestable and asserted the wrong thing. |
| `capability_conformance.rs` BigQuery `supports_transactional_ddl` | `true` → `false`, with the doc quote. |
| `example_diagnostics/smoke_and_migration.rs::github_activity_no_diagnostics` | The `KeyedFold` downgrade no longer fires; its absence is the documented assertion. |

## Spec delta

- `docs/specs/state.md` §"Which dialects realise which structure" — BigQuery's
  reconciliation-ledger row is `yes`; a fourth load-bearing fact names the zero-row-abort
  mechanism and states the isolation property it depends on; a new paragraph records the one
  shape BigQuery refuses (first-run DDL inside the fold's transaction) and its remedy.
- `docs/specs/incremental_models.md` §Constraints "Never fold a delta already reflected in the
  state" — the guarantee is now dialect-plural: the ledger *refuses* rather than reports, a
  separate existence probe is named as an inadmissible realisation, and the two admissible
  mechanisms (enforced key; conditional record with a zero-row abort, sound only where the
  engine refuses concurrent mutating transactions on the ledger table) are stated underneath.

## Gates

| Gate | Result |
|---|---|
| `bash .claude/scripts/verify-phase.sh` | see the report below |
| `cargo test -p smelt-runtime --test state_guard_census` | 3 passed |
| `cargo test -p smelt-runtime --test availability_seam` | see the report below |
| `cargo test -p smelt-logical --test maintenance_availability` | 22 passed |
| `cargo test -p smelt-logical --test maintenance_dialect_blindness` | 3 passed |
| `cargo test -p smelt-state --test ledger_dialect` | 7 passed |
| `cargo check -p smelt-cli --features bigquery` | clean |
| `bash .claude/scripts/large-file-check.sh` | OK — no baseline raised |

No live warehouse was reached.

## What phases 15 and 16 inherit

- **Row 15** (tombstone ledger) is now the *only* structure keeping a raw `STATE-GUARD` in the
  census, at `succession/execute.rs:98,313`. Both ledger rows above it are done, and
  `ledger_fold_record_sql`'s dispatch is the pattern for any statement whose refusal semantics
  differ per dialect. It also inherits the DDL question in its own form: whatever transaction
  the tombstone patch opens must not contain permanent-entity DDL either, and
  `supports_transactional_ddl` is now the honest flag to ask.
- **Row 16** inherits four live checks, two of them new here:
  1. that a repeat fold really refuses on the real engine, and that the sentinel survives the
     adapter's error envelope unmangled (`is_already_reflected` is a `contains`, so partial
     mangling is tolerated — total rewriting is not);
  2. that `@@row_count` after a `MERGE … WHEN NOT MATCHED` really is `0` on a repeat;
  3. phase 13's inherited question, now **answered from the docs rather than open**: BigQuery
     does not accept permanent-entity DDL inside a transaction. This phase acts on that for the
     additive fold. It does **not** act on it for the *idempotent* merge-ledger path, which is
     a live exposure worth naming: `driver.rs`'s `Grade::Idempotent` arm passes the first
     (table-creating) step's `CREATE TABLE … AS` to `execute_write_with_bookkeeping` alongside
     a ledger upsert, and `write_with_bookkeeping_plan` puts both in one transaction. On the
     docs' reading that script will be rejected by the engine — loudly, not silently, and only
     on a first run. The fix is local to `write_with_bookkeeping_plan` (or to that call site),
     and it was left out of this phase deliberately rather than by oversight: it is a different
     grade, a different seam, and changing it here would have meant new untested behaviour on a
     path this row does not touch;
  4. that the `MERGE`-as-upsert really no-ops on a repeat window (inherited unchanged from
     phase 13).
