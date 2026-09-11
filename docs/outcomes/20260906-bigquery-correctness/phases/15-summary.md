# Phase 15 summary — the tombstone ledger is realised on BigQuery

**Row:** 15 · **Criteria served:** 8, 9, 3, 7 · **Status:** done (row flipped, not refused)

## The §1 verdict, construct by construct

The plan's first instruction was to enumerate what the two blocked emitters emit and check
each against GoogleSQL **before** writing anything. Eleven constructs; three needed a
dialect branch, one needed a type-mapping route, and seven did not need touching. Stating
the "no change needed" half is the point — each one was a candidate for a gratuitous
divergence.

| Construct | GoogleSQL verdict | What happened |
|---|---|---|
| `(k…) IN (SELECT k… FROM (<batch>))` — touched-key scoping, twice per domain | **Rejected.** GoogleSQL has no row constructor; `(a, b)` is a parenthesised expression, not a tuple, so a multi-column `IN` subquery is a syntax error. A *single*-key model would have parsed — the worst kind of failure, silent until someone declares two key columns. | Correlated `EXISTS` over an aliased relation on BigQuery; the relation aliases (`__smelt_presented`, `__smelt_tombstones`) exist only on that path, because the `IN` form needs none. |
| `QUALIFY ROW_NUMBER() OVER (…) = 1` — the dedup relation | **Unsettleable offline.** GoogleSQL has `QUALIFY`, but its preconditions in this exact position cannot be established without a warehouse. | Replaced with an explicit `ROW_NUMBER() … WHERE __smelt_dedup_rn = 1` — the same relation, universally valid, and the shape `emit_succession_full_rebuild`'s own fold already used. |
| `WITH __smelt_domain AS (…)` inside the `MERGE`'s `USING` subquery | **Unsettleable offline** for the same reason. | Nested derived tables on BigQuery. |
| `DELETE FROM <ledger>` (rebuild) | **Rejected.** GoogleSQL requires a `WHERE` on every `DELETE`. | `DELETE FROM … WHERE TRUE` on BigQuery; DuckDB keeps the bare form. |
| Ledger column types via `DataType::to_sql()` | **Rejected.** `VARCHAR`/`INTEGER` are `Type not found`. | Routed through `ddl_bigquery::bigquery_type_sql`, whose `Err` is propagated with the column named. |
| `PRIMARY KEY (k…, t)` | **Rejected** bare. | `NOT ENFORCED` on BigQuery — and nothing depends on it, because the tombstone insert's idempotence is its own anti-join. |
| `INSERT INTO t (cols) SELECT … WHERE <flag> AND NOT EXISTS (correlated)` | **Accepted.** | **Unchanged, and asserted byte-identical to DuckDB's** — the strongest available statement that this construct needed no branch. |
| `MERGE INTO t AS target USING (…) AS source ON … WHEN MATCHED THEN UPDATE SET … WHEN NOT MATCHED AND <cond> THEN INSERT (…) VALUES (…)` | **Accepted.** GoogleSQL's `MERGE` has all four clauses, including the `AND` on the not-matched arm. | Unchanged. The one-source-row-per-target-row requirement BigQuery enforces is already met by the dedup relation. |
| `UNION ALL` of three arms, `TRUE`/`FALSE` literals, `LEAD`/`LAG` over a partition | **Accepted.** | Unchanged. |
| `SELECT *, ROW_NUMBER() OVER (…) AS rn` (rebuild fold) | **Accepted.** | Unchanged. |
| `CAST(x AS STRING)`, `||`, `STRING_AGG`, `COUNT(DISTINCT …)` (clock-tie probe) | **Accepted**, and already dialect-dispatched by `probes::probe_dialect_string_type` / `clock_tie_sample_agg` before this phase. | Unchanged; only the domain beneath the probe changed. |

**One construct is accepted on documented coercion rules rather than measurement, and is
named for phase 16**: the tombstone arm of the domain union projects a bare `NULL AS
<payload col>`. GoogleSQL's coercion rules state an untyped `NULL` literal is coercible to
any type and a set operation computes a supertype, so this should carry the presented
arm's column type — but the emitter holds only payload column *names*, not types, so it
cannot emit `CAST(NULL AS <t>)` without a new input. If BigQuery rejects it, the fix is to
thread payload types into `emit_succession_patch`, not to change the relation.

## What landed

**The two `assert!`s are gone, and the refusal they were is now typed.**
`emit_succession_patch` and `emit_succession_full_rebuild`
(`crates/smelt-logical/src/maintenance/emit/succession/mod.rs`) return
`Result<StatementGroup, UnsupportedSuccessionDialect>`. Only Spark is refused, and its
absence is stated as permanent (no cross-table Delta transaction) rather than pending —
the same sentence `smelt_state::ledger` already carries. The internal helpers keep
exhaustive `match`es with Spark sharing DuckDB's arm and a comment saying it never reaches
there, so a fourth dialect is still a compile error.

**The split the maintenance-plan purity rule forces.** The tombstone table's *DDL* is
bookkeeping and went to `smelt-state`:
`crates/smelt-state/src/ddl_bigquery/tombstone.rs` (new) plus
`crates/smelt-state/src/tombstone.rs` (new), the `SqlDialect`-keyed exhaustive dispatch —
third instance of the `ledger.rs`/`observed_delta.rs` pattern, Spark refused by name.
Every *statement* stayed single-owned in `smelt-logical` and became dialect-plural there.
`statement_parity`'s structural no-authoring leg is what proves the second half; no
exclusion had to be touched, because `smelt-logical/src/maintenance/emit/` and
`smelt-state/src/ddl_bigquery/` are both already directory exclusions.

**`bigquery_type_sql`'s `Result` propagates all the way to the driver.**
`generate_tombstone_table_ddl` returns `Result<String, UnmappableTombstoneColumn>`; the
dispatch wraps it in `TombstoneDdlError::UnmappableColumn`; `succession/execute.rs`'s two
`ensure_sqls` sites `?` it. A `MAP` key column now fails with the column name and the
reason rather than a substituted type the next fold could not write.

**The two guards are gone, not annotated.** `succession/execute.rs:98,313`'s
`backend.dialect() != SqlDialect::DuckDB` bails are now
`!maintenance_driver::realises_tombstone_ledger(backend.dialect())`, derived from
`realisable_state_structures` exactly as phases 13 and 14 did. The `SqlDialect` import is
gone from that file. **The census guard list is now empty** — every structure's run-layer
gate is derived.

**Two DuckDB-only call sites in the same file were routed through the dispatches** phase
13 deliberately left behind: `ddl_duckdb::generate_ledger_table_ddl` and
`generate_ledger_upsert_sql` became `state_ledger::ledger_table_ddl` /
`ledger_upsert_sql`. They were unreachable off DuckDB while the guard stood; with the
guard gone they would have emitted DuckDB SQL into a BigQuery job.

**The row flipped**, and the legible result is a real workspace:
`examples/github_activity`'s two `SuccessionPatch` cells (`raw.github_events`,
`raw.github_events_arrival`) no longer downgrade on the `bigquery` target, so
`github_activity_no_diagnostics` is back to `check_workspace_no_diagnostics` — the first
time since the `bigquery` target was added that the workspace is diagnostic-clean. Every
structure the workspace's cells need is now realised on both engines, which is criterion
9's claim stated as an absence.

## Proving an empty census still fails closed

The plan flagged this explicitly, and it needed real work: with zero guards,
`every_duckdb_guard_names_an_unrealisable_structure` iterates an empty list and passes no
matter what its body says. The verdict logic was extracted into a pure
`judge(Vec<Guard>) -> Verdicts`, and non-vacuity now stands on three legs, of which the
third is new:

1. the scanned file set is non-empty (`maintenance_driver_sources` already asserted this);
2. the scanner recognises a guard when one exists
   (`the_scanner_distinguishes_guards_from_prose`, pre-existing);
3. **`an_empty_census_still_fails_closed_on_a_planted_guard`** — the same `judge` the real
   test calls, driven on three planted guards: unannotated, unknown-structure, and one
   annotated `MergeLedger`/`TombstoneLedger` (both now realisable on BigQuery, so both are
   contradictions). If `judge` were stubbed to `Verdicts::default()` the real test would
   still pass and this one would fail.

`the_census_is_non_empty_and_covers_the_known_guards` became
`the_census_is_empty_because_every_gate_is_derived`, asserting the empty list *is* the
finished state and that none of the four structures reacquires a raw guard.

## Divergences from the plan

- **The plan expected `succession.rs` or `ddl_bigquery.rs` to tip the line ratchet.**
  Neither did in the end, but `succession.rs` (983 lines, cap 1500) would have with the
  new tests, so it was split pre-emptively into `emit/succession/{mod.rs,tests.rs}`
  (680 + 660). `ddl_bigquery/` was already a directory and gained
  `tombstone.rs` (164). No baseline entry was raised. **Splitting turned a different gate
  red than phase 12 warned about**: not `statement_parity` (whose exclusions are
  directory-shaped and already covered `emit/`) but
  `maintenance_dialect_blindness`, whose `#[cfg(test)] mod ... { ... }` stripper cannot
  reach a test module that now lives in its own file. Corrected the same way
  `state_guard_census` and `statement_parity` already do it: skip `tests.rs` files and
  `tests/` directories in the walk, documented in the module header.
- **The plan listed `emit_succession_union_relation` as an emitter to check.** No function
  of that name exists; the union is built by the private `build_domain_cte`, shared by the
  patch and the clock-tie probe. Making it dialect-plural therefore fixed the probe at the
  same time, which is why the probe needed no separate change.
- **The rebuild group's atomicity degrades on BigQuery rather than being refused**, and the
  reason it differs from phase 14's refusal is specific to the statement. Phase 14 refused
  a first-run additive fold because a ledger row surviving a failed create would claim a
  fold that never happened. The rebuild has no such asymmetry: it is a pure function of
  the whole retained source, both tables are re-derived from scratch, and a partial
  application is repaired by re-running. `write_with_bookkeeping_plan`'s existing
  `NothingToBind`/`NonAtomicCreatingWrite` vocabulary already handles it — no new code,
  but it is now stated in `docs/specs/state.md` and in the emitter's doc comment rather
  than being an unwritten consequence.

## Tests

| Test | What it holds |
|---|---|
| `smelt-logical` `emit::succession::tests` (+7) | The BigQuery patch group's two statements **verbatim**; the tombstone insert asserted byte-identical to DuckDB's (with the `MERGE` asserted *different*, so it is not vacuous); the three avoided constructs asserted absent on BigQuery **and present on DuckDB**; the BigQuery rebuild group verbatim; DuckDB's bare `DELETE` preserved; the clock-tie probe's `EXISTS` domain and `STRING` casts. |
| `smelt-logical` `emit::succession::tests` (2 corrected) | The two `#[should_panic]` tests became typed-refusal tests: Spark refused by name, **both** DuckDB and BigQuery succeed. |
| `smelt-state` `ddl_bigquery::tombstone::tests` (3, new) | The GoogleSQL DDL verbatim (`INT64`/`STRING`, backticked path, `NOT ENFORCED`); the unmappable-type refusal names the column; the drop DDL. |
| `smelt-state --test ledger_dialect` (+5) | Dispatch exhaustive, Spark refused by name; no DuckDB spelling on the BigQuery path; DuckDB delegates verbatim; unmappable column refused with the column named; non-vacuity between the two realising dialects. |
| `smelt-runtime --test state_guard_census` (1 new, 1 rewritten) | The empty census, and the planted-guard control above. |
| `maintenance_availability` (2 corrected, 1 new) | `tombstone_ledger_is_realisable_everywhere_but_spark`; the downgrade test drops BigQuery and keeps Spark + `warehouse_tables: none` for non-vacuity; `bigquery_keeps_the_succession_patch_technique` asserts the **absence** of the downgrade. |
| `example_diagnostics` (corrected) | `github_activity_no_diagnostics` is a clean-workspace assertion again. |

**The red was the right red.** Pointing the BigQuery `using_select` back at the DuckDB
branch fails `the_bigquery_patch_avoids_the_constructs_googlesql_lacks` on all three
constructs at once and the verbatim test with a diff at the `USING` clause — not an
incidental formatting mismatch.

## Spec delta

- `docs/specs/state.md` §"Which dialects realise which structure": the tombstone-ledger row
  is `yes` for BigQuery; four load-bearing facts of that realisation (the `EXISTS` scoping,
  the nested-derived-table dedup, the `WHERE TRUE` delete, the typed-or-refused columns),
  prefaced by the two-layer split the purity rule forces; and the rebuild's atomicity
  degradation with the argument for why it is admissible here and was not for the additive
  fold.
- `docs/specs/incremental_shapes.md` §"The tombstone ledger (hidden state)": "Physical
  shape" now states the advisory-key and refuse-an-unmappable-type rules as properties of
  the shape rather than one dialect's spelling; "Lifecycle" widens "never authored by a
  backend" from the rebuild `SELECT` to every statement the ledger participates in, in
  whichever dialect, and records the unbound-rebuild degradation.

## Gates

| Gate | Result |
|---|---|
| `bash .claude/scripts/verify-phase.sh` | ALL GREEN |
| `cargo test -p smelt-runtime --test statement_parity` | 16 passed |
| `cargo test -p smelt-runtime --test state_guard_census` | 4 passed |
| `cargo test -p smelt-runtime --test availability_seam` | 6 passed |
| `cargo test -p smelt-runtime --test observed_delta` | passed |
| `cargo test -p smelt-backend-bigquery --test never_fold_twice` | 3 passed |
| `cargo test -p smelt-logical --test maintenance_availability` | 23 passed |
| `cargo test -p smelt-logical --test maintenance_dialect_blindness` | 3 passed |
| `cargo test -p smelt-dialect --test emission_ownership` | 11 passed |
| `cargo test -p smelt-state --test ledger_dialect` | 17 passed |
| `cargo check -p smelt-cli --features bigquery` | clean |
| `bash .claude/scripts/large-file-check.sh` | OK — no baseline raised |

No live warehouse was reached.

## What phase 16 inherits

Row 16 now carries six live checks, three of them new here:

1. **The untyped `NULL` in the domain union.** Does GoogleSQL coerce a bare `NULL AS
   <payload col>` to the presented arm's column type across a three-arm `UNION ALL`? The
   docs say yes; nothing offline can prove it. If not, thread payload *types* into
   `emit_succession_patch`.
2. **The `MERGE` with a nested-derived-table `USING`.** The one statement in this phase no
   offline test can execute. Its two riskiest sub-constructs were removed rather than
   guessed at, but the whole statement has never run.
3. **The rebuild's three unbound jobs.** Confirm BigQuery really does reject the
   transactional form (the reason the plan degrades) and that the unbound form leaves the
   presented table and the ledger consistent after a clean run.
4. Phase 14's three inherited checks, unchanged: the repeat-fold refusal and its sentinel
   surviving the adapter's error envelope; `@@row_count = 0` after a repeat `MERGE … WHEN
   NOT MATCHED`; and the idempotent merge-ledger path's first-run `CREATE TABLE … AS`
   inside `write_with_bookkeeping_plan`'s transaction, which phase 14 named as a live
   exposure and deliberately did not fix.

It also inherits a *smaller* job than expected on the plan side: `examples/github_activity`
is diagnostic-clean on both targets, so the live re-run compares one plan on two engines
with no downgrade to explain away.
