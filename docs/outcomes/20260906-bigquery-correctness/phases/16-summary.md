# Phase 16 summary — the live BigQuery run that closes the reopening

**Row:** 16 · **Criteria served:** 9, 5, 4, 7, 3 · **Status:** done

## The run

Nine live runs against `smelt-bq-test-20260816.smelt_dogfood`, driven by
`source scripts/bq-dogfood-env.sh` plus the impersonated ADC token the adapter needs
(`SMELT_BQ_ACCESS_TOKEN=$(gcloud auth application-default print-access-token)` — the one
piece of setup written down only in the spine's phase-11 summary). `githubarchive` was
never touched and `scripts/bq-dogfood-loader.sh` was not re-run.

The headline pair, both at the **default** `--jobs` (the host's core count), both green:

```
smelt run --target bigquery --start 2026-08-06 --end 2026-08-07 \
  -e silver.actor_sessions -e marts.daily_active_contributors
```

| Run | Report id | Outcome | Wall |
|---|---|---|---|
| A (first) | `20260911-121243-1453d8` | 14 success / 0 failed / 0 skipped | 108.3s |
| B (repeat of the same window, against A's state) | `20260911-121439-a3b26c` | 14 success / 0 failed / 0 skipped | 107.6s |
| D (confirmation on the **committed** tree, after the `ledger_reset` extraction) | `20260911-130933-e9f8e4` | 14 success / 0 failed / 0 skipped | 123.7s |

That matches the 2026-09-11 baseline's **14 models** exactly. Without the two exclusions the
run is 11 success / 1 failed / 4 skipped (`20260911-121054-a421d3`), the same shape the
baseline session's mid-runs had: `silver.actor_sessions` still fails at **compile time** with
`UnsupportedOnBackend` naming `LAG` and `MAX` under an INTERVAL `RANGE` frame, before any
warehouse round trip, and takes its one downstream with it. Unchanged by design.

The wall time is 108s against the baseline's 38s. That is not a regression to hunt: it is the
ledger gate (defect E below) serialising this run's ledger-mutating transactions, which is
what BigQuery's own isolation rule charges for the never-fold-twice guarantee.

**Cost.** Every run and every verification query billed well under a cent. The heaviest read
was the `INFORMATION_SCHEMA.TABLES` listing at 10 MB (BigQuery's metadata-query minimum); every
state read-back was 0–675 bytes, and the pipeline's own reads are the same ~20 KB fixture the
prior runs used. No large scan was needed and none was run.

## Four defects, all fixed and gated offline

Every one of them passed every offline gate in the tree before this run. All four are in the
maintenance/bookkeeping layer — none is a registry spelling, which is why the BigQuery gap
ratchet holds rather than falls.

**C — `CAST(<key> AS VARCHAR)` in the driver's changed-key projection.** GoogleSQL has no
`VARCHAR` (`Type not found: VARCHAR`), so **every** keyed model with a change-suppressed write
failed its observed-delta record on BigQuery — `silver.events_deduped` first. The cast type now
comes from `smelt_logical::maintenance::emit::probe_dialect_string_type(dialect)`, the single
owner of that spelling, threaded through `changed_keys_select`,
`keyed_fold_changed_keys_select`, `staged_candidate_changed_keys_select` and the
`WindowedKeyedRule::observed_delta_changed_keys_sql` seam. `repair_keys_literal_select`'s
empty-keys relation had the same hardcode while already *taking* a dialect, and is fixed with
it.

**D — `<col> >= DATE '<start>'` in the succession step's window predicate.** DuckDB widens
`DATE` to `TIMESTAMP` implicitly; GoogleSQL refuses outright (`No matching signature for
operator >= for argument types: TIMESTAMP, DATE`), which took out both succession models'
clock-tie probes. The bounds are untyped string literals now
(`succession_window_predicate`), which both dialects coerce to the column's own type, so one
spelling serves a `DATE` and a `TIMESTAMP` partition column.

**F — a DuckDB ledger `CREATE TABLE` reached from `execute/project/mod.rs`.** Two direct calls
to `smelt_state::ddl_duckdb::generate_ledger_{table_ddl,recompute_reset_sqls}` in the region-
recompute reset path. Unreachable-and-harmless while the reconciliation ledger was DuckDB-only;
DuckDB SQL in a BigQuery job the instant phase 13 declared it realisable there. Both route
through `smelt_state::ledger` now. This is the *third* phase to take this exact fix (13 took
two call sites, 15 took two more), which is why it is now gated structurally rather than by
inspection.

**E — the one this run could not have been predicted into finding.** BigQuery cancels a
multi-statement transaction that mutates a table another in-flight transaction is also mutating
("Transaction is aborted due to concurrent update against table … `_smelt_ledger`"). Every
maintained model's bookkeeping transaction mutates that one table, so a parallel run loses
models at random — and the write-conflict detection doing it is *precisely* the mechanism phase
14's decision log named as the soundness argument for the additive never-fold-twice refusal on a
dialect whose `PRIMARY KEY`s are `NOT ENFORCED`. The guarantee and the failure are the same
fact. Fixed in two halves:

- **Classification.** A concurrent-update abort is now `BackendError::TransactionConflict`,
  classified **transient** — the engine rolls the whole transaction back before cancelling it,
  so re-issuing the identical statement group is the engine's own documented remedy, and the
  ordinary bounded retry performs it. It is the only error class in `is_transient`'s exhaustive
  match whose remedy is retry by definition rather than by hope.
- **Avoidance.** Retry alone was measured insufficient (three attempts at 200/400/800 ms lose to
  a multi-second script job): `BigQueryBackend::ledger_gate` serialises *this process's* ledger
  transactions, so a run does not race itself. Deliberately process-scoped — a second writer
  still conflicts, and that is what the transient classification is for.

## The eight inherited checks

| # | Check | Verdict |
|---|---|---|
| 1 | The untyped `NULL` coercion in the domain union (phase 15) | **verified** |
| 2 | The patch `MERGE` as a whole (phase 15) | **verified** |
| 3 | The transactional rebuild is really rejected, unbound form consistent (phase 15) | **not exercised** |
| 4 | `SMELT_LEDGER_ALREADY_REFLECTED` survives the adapter's error envelope (phase 14) | **not exercised** |
| 5 | `@@row_count = 0` on a repeat `MERGE` (phase 14) | **not exercised** |
| 6 | First-run DDL inside `write_with_bookkeeping_plan`'s transaction (phases 13/14/12) | **not exercised** |
| 7 | The Arrow list shape for a `REPEATED STRING` (phase 12) | **verified** |
| 8 | `ARRAY<STRING>` without `NOT NULL` accepts a fully-suppressed window's row (phase 12) | **verified** |

**1 — verified.** `build_domain_cte`'s tombstone arm projects a bare `NULL AS <payload col>`
into a 3-arm `UNION ALL`. BigQuery type-checks a set operation at plan time, before any row
flows, so the clock-tie probe **running at all** on `silver.repo_naming` and
`silver.actor_naming` is the measurement: GoogleSQL coerced the untyped `NULL` to the set
operation's supertype exactly as phase 15's reading of the coercion rules said it would. No
`CAST(NULL AS <t>)` is needed and no payload types need threading. The one honest caveat: this
source produces no deletes, so the arm type-checked but carried zero rows — what is proven is
the coercion, not the delete path's values.

**2 — verified.** Both succession models executed `emit_succession_patch`'s `MERGE` on runs A
and B and again on the un-excluded run, for the first time anywhere. Read back from the
warehouse: `silver_repo_naming` and `silver_actor_naming` each hold 6,053 rows and their
`__tombstones` tables exist and hold 0 (no deletes in this source). The nested-derived-table
dedup relation phase 15 chose over `QUALIFY`, the correlated-`EXISTS` `touched_keys_predicate`
it chose over the row-constructor `IN`, and the `DELETE … WHERE TRUE` are all accepted.

**3 — not exercised, and the reason is a different guard firing first.**
`smelt run --target bigquery --full-refresh -s silver.repo_naming -s silver.actor_naming
-s silver.events_deduped --start 2026-08-04 --end 2026-08-08` is refused before any backend
call: `SourceRetentionExceeded` — "a whole-table recompute reaches past every finite bound, but
stored output already exists and no license was given for `raw.github_events` (retains 3888000
seconds)". `rebuild_succession_state` is therefore unreachable on this dataset without an
explicit license flag, and passing one to chase a check would have rewritten the long-lived
dogfood state the equivalence invariant is measured against. Recorded unverified rather than
forced. The cheap safe route for a future phase is the integration suite's **ephemeral**
dataset, where no stored output exists and the guard does not fire.

**4, 5 — not exercised, and the same single fact explains both.** No cell in
`examples/github_activity` grades `Grade::Additive`. That is not an assumption: run B replayed
run A's window byte-for-byte against A's committed ledger, and an additive cell would have
refused it (`driver.rs`'s `Grade::Additive` arm turns `AlreadyReflected` into a hard
`KeyedReprocessedWindow` bail). Run B succeeded on all 14 models, so no additive fold was
reached, so neither the sentinel's trip through the adapter's envelope nor `@@row_count = 0`
after a repeat `MERGE … WHEN NOT MATCHED` was executed. Both remain proven offline only
(`smelt-backend-bigquery`'s `never_fold_twice` suite). What the run *did* prove about the same
table is the idempotent half: `_smelt_ledger` holds exactly **9 rows** after two runs of the
same window, one per maintained model, with no duplicate — the `MERGE … WHEN NOT MATCHED`
record is idempotent live, as phase 13 designed it.

**6 — not exercised.** Every target in `smelt_dogfood` already existed, so no write group was a
`CREATE TABLE … AS` and `BookkeepingAtomicity::NonAtomicCreatingWrite` never fired (no
`warn!` line in any run). The path that would create one is `--full-refresh`, which check 3's
retention guard refuses. Same remedy: an ephemeral dataset.

**7 — verified.** `read_observed_delta` decoded `silver_events_deduped`'s row on the runs that
followed it, through `decode_string_list_column`, which phase 12 rewrote to **error** on any
unrecognised Arrow shape rather than silently returning empty. A green run is therefore the
evidence: the adapter's `REPEATED STRING` → Arrow mapping is one of the list/string width
combinations the decoder accepts.

**8 — verified, by reading it back.**

```
model_name              window_start  window_end   n_keys  n_parts
silver_events_deduped   2026-08-06    2026-08-07   0       0
```

One row, present, with both `ARRAY<STRING>` columns empty — a fully-suppressed window (run B
changed nothing) landing exactly the "present-and-empty, never absent" shape phase 12's
empty-versus-absent argument depends on, and the columns without `NOT NULL` accepted the write.

## Cross-target agreement (criterion 5)

No new divergence was registered and none was tolerated. All four defects were *fixed*, not
registered: each was smelt emitting DuckDB SQL to BigQuery, which is a bug with a right answer,
not a difference between engines that needs a reasoned entry. The unexplained-difference count
stays zero. Dual-target **value** parity is untouched here and remains the spine's phase 13 —
the BigQuery leg holds three days and the DuckDB fixture thirty, so equal row counts are not
expected and nothing in this phase compares the two populations.

## Gates

| Gate | Result |
|---|---|
| `bash .claude/scripts/verify-phase.sh` | ALL GREEN |
| `cargo test -p smelt-db --test dialect_audit` | pass (61 tests; coverage table regenerated, no diff) |
| `cargo test -p smelt-runtime --test statement_parity --test state_guard_census` | pass |
| `cargo check -p smelt-cli --features bigquery` | clean |
| `bash .claude/scripts/large-file-check.sh` | OK |
| `cargo test -p smelt-runtime --test maintenance_sql_dialect_purity` (new) | 8/8 |
| `cargo test -p smelt-backend-bigquery` (incl. new `--test ledger_gate`) | pass |

**New gates, one per defect class:**

- `crates/smelt-runtime/tests/maintenance_sql_dialect_purity.rs` — behavioural legs (each
  fragment rendered for BigQuery *and* DuckDB, so neither can pass by ignoring its dialect
  argument) plus three structural scans over `src/maintenance_driver/`: no cast to a type
  GoogleSQL has no name for, no typed `DATE '…'`/`TIMESTAMP '…'` literal, and no multi-dialect
  state structure reached by its named per-dialect builder (defect F's class, scanning all of
  `src/`). Each scan has a planted-offence control so it cannot pass vacuously, and a
  `DIALECT-OK:` waiver for a genuinely single-dialect site.
- `crates/smelt-backend-bigquery/tests/ledger_gate.rs` — every seam that opens a transaction
  over `_smelt_ledger` acquires `ledger_gate`, with a control proving the body extractor is
  scoped to one method rather than grepping the file.
- `crates/smelt-backend-bigquery/src/sql.rs` — the conflict classifier, with its negative
  control (an ordinary SQL error stays deterministic).

**Two pre-existing tests encoded the old claim and were corrected, not deleted.**
`maintenance_driver::tests::repair_keys_literal_select_empty_keys_is_dialect_independent`
asserted a hardcoded `VARCHAR` on all three dialects — a claim that was simply false and would
have failed at the warehouse the first time a repair with no affected keys ran on BigQuery. It
is now `..._is_well_typed_per_dialect`, asserting each dialect's own string type. And
`statement_parity::succession`'s fixture restated the typed `DATE '…'` window predicate as a
literal; it now calls `succession_window_predicate`, the same single owner the driver does, so
expectation and driver cannot drift apart again.

## Ratchets

- `.claude/dialect-gaps-baseline.txt` — `dialect_gaps_bigquery` **held at 42**, with a dated
  note. All four defects were in the maintenance/bookkeeping layer, so none gives any of the 42
  no-verdict registry entries a verdict; the two constructs the run met at the registry boundary
  (`LAG`/`MAX` under an INTERVAL `RANGE` frame) are already clause-level
  `Emission::Unsupported` refusals, not gap rows. `dialect_gaps_duckdb` 4 and
  `dialect_gaps_spark` 4 unchanged.
- `.claude/parser-gaps-baseline.txt` — `duckdb_seed_gaps 0`, unchanged.
- `.claude/large-file-baseline.txt` — unchanged; no file split was needed.

## Residual, named rather than glossed

- **`repair_keys_literal_select` escapes a string literal DuckDB-style** (`'` → `''`), which
  does not continue a GoogleSQL string, while a backslash *is* an escape character there — the
  portability trap phase 13 fixed for the ledger via `ddl_bigquery::escape_string_literal`. This
  path is not reached by `github_activity` and was not exercised live, so it is recorded here
  rather than fixed under a closing row (criterion 2). It wants the same one-line treatment the
  ledger got, in a phase that can test it.
- **Checks 3, 4, 5 and 6 need an ephemeral dataset, not this one.** All four are blocked by the
  same two properties of a long-lived dogfood dataset: stored output exists (so the retention
  guard refuses a whole-table recompute) and every target exists (so no write group creates
  one). None is blocked by a defect.
