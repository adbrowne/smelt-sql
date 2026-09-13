# Phase 6c summary — drop-type-mismatch recognised; full refresh now 11/16, one new blocker found

## Shipped

- `crates/smelt-backend-spark/src/lib.rs`: two pure, tested predicates —
  `is_table_not_view_error` and `is_view_not_table_error` — replace the inline
  `msg.contains(...)` chains in `drop_view_if_exists`, `drop_table_if_exists`, and the
  DROP-TABLE-falls-back-to-DROP-VIEW branch of `create_table_as`. Each now recognises three
  message shapes: the two vanilla-OSS-Spark ones already handled
  (`WRONG_COMMAND_FOR_OBJECT_TYPE`, the `is a VIEW` / `DROP VIEW requires a VIEW` text) plus
  Unity Catalog's own `[DROP_COMMAND_TYPE_MISMATCH]` text in both directions
  ("Cannot drop a table with DROP VIEW" / "Cannot drop a view with DROP TABLE").
- `crates/smelt-backend-spark/src/tests.rs`: the five named tests
  (`drop_view_mismatch_recognises_databricks_message`,
  `drop_view_mismatch_recognises_oss_spark_messages`,
  `drop_table_mismatch_recognises_databricks_message`,
  `drop_table_mismatch_recognises_oss_spark_messages`, `unrelated_error_is_not_swallowed`) —
  all green, no regression on the existing 31 tests in the crate.
- A live full-refresh run against the dogfood schema
  (`smelt run --target databricks --full-refresh --allow-full-refresh
  --event-time-start 2026-08-05 --event-time-end 2026-08-07`), run id
  `20260912-103014-1e03d5`, report at
  `examples/github_activity/.smelt/targets/databricks/reports/20260912-103014-1e03d5.json`
  (not committed — `.smelt/` is gitignored repo-wide and no prior phase in this outcome has
  force-added a report either; relevant contents are quoted below instead, following phase
  6/6b's own precedent).

## Result: 11 success / 1 failed / 4 skipped (16 total)

The three self-referential bootstrap models that failed in 6b's re-run
(`bronze.events`, `silver.actor_naming`, `silver.repo_naming`) now succeed — the
`DROP_COMMAND_TYPE_MISMATCH` fix resolved exactly the condition it targeted, live. Criterion
6's first half ("the model set has never completed a full refresh") is not yet fully closed:
one model fails on its own defect, unrelated to drops.

**New finding — `CAST(x AS VARCHAR)` with no length is rejected by Databricks/Unity Catalog.**

- Model: `silver.actor_sessions`.
- Statement: `... CONCAT(CAST(actor_id AS VARCHAR), '-', CAST(session_start_ts AS ...` (line 93
  of the compiled SQL).
- Live error: `[DATATYPE_MISSING_SIZE] DataType "VARCHAR" requires a length parameter, for
  example "VARCHAR"(10). Please specify the length. SQLSTATE: 42K01`.
- Root cause: Spark/Databricks' SQL parser treats bare `VARCHAR` (no length) as the
  `char/varchar` type family, which Spark's own docs mark set-only for read compat and reject
  outright as a cast target with no length — unlike DuckDB and BigQuery, which both accept an
  unbounded `VARCHAR`/`STRING` cast. This is a dialect emission gap: the printer emits
  `VARCHAR` unconditionally for a string cast rather than dispatching a Spark/Databricks-only
  length (e.g. defaulting to `STRING`, which Spark treats as unbounded and every version
  accepts) through the same per-dialect emission path the Function-registry invariant already
  requires for other type spellings.
- Fix candidate: give cast-target type printing the same per-dialect emission treatment as
  `BuiltinRegistry` gives function spellings — on `SqlDialect::Spark`/`Databricks`, print an
  unqualified string cast as `STRING` rather than `VARCHAR`. Likely lands in
  `crates/smelt-dialect/src/printer/` wherever `CAST ... AS <type>` is rendered; needs its own
  test fixture rather than living inside this phase's drop-mismatch boundary.
- Not fixed here: only 1 of 16 models is blocked by it (11 succeed), so the outcome's "only fix
  what's needed for a run to complete at all" exception does not apply — a run *did* complete,
  just not cleanly. Recorded for the criterion-9 findings handoff (phase 10) and as row 7's
  known starting gap.
- Downstream skips (4): `models/silver/events_deduped.sql` and
  `models/marts/daily_active_contributors.sql` both reference `actor_sessions` directly (or
  transitively), accounting for the 4 skipped models in the report's `outcome_counts`.

## Decisions

- Followed the plan's naming exactly (`is_table_not_view_error` / `is_view_not_table_error`)
  and kept the direction each predicate serves matching the existing call sites' comments —
  no behavior change to the OSS-Spark-only legs, confirmed by the regression tests
  (`drop_view_mismatch_recognises_oss_spark_messages`, `drop_table_mismatch_recognises_oss_spark_messages`).
- Did not attempt the `VARCHAR` length fix in this phase — it is a distinct dialect-emission
  defect (cast-target type spelling, not drop recognition) and fixing it here would blur the
  phase's own stated boundary and its "only fix what's needed to complete at all" exception,
  which does not apply since the run completed for 11/16 models.
- Spark parity tier confirmed untouched by inspection: no change to `spark_adapter.py`,
  `sql.rs`, or any emitted statement — only the message-recognition predicates changed, and
  OSS Spark's own message shapes are still matched identically.

## For the next planner

- **Row 7 (three consecutive incremental windows) should start from this baseline** — 11/16
  models, `silver.actor_sessions` and its 3 downstream dependents unresolved — using the exact
  command above (`--target databricks --full-refresh --allow-full-refresh --event-time-start
  2026-08-05 --event-time-end 2026-08-07` for the initial full refresh, then advancing
  `--event-time-start`/`--event-time-end` day by day for each incremental window). Row 7 will
  need to either fix the `VARCHAR` cast-length defect first (a small, isolated dialect-printer
  change with its own test) or accept `actor_sessions` and its 3 dependents as excluded from
  the three-window and dual-target-parity checks — the planner should decide which, since
  criterion 7's "each model's output... or the difference is a registered divergence" wording
  suggests the former is cheaper than carrying an unregistered gap into rows 7-9.
- The `VARCHAR`-length fix candidate above is scoped small enough it could be its own row
  ahead of row 7, mirroring how 6b and 6c each closed one dialect-recognition gap before the
  live windows began.
- Nothing else from this phase's scope surfaced further findings — the drop-mismatch fix was
  clean on the first live run.

## Gates

- `cargo test -p smelt-backend-spark --lib --quiet` — 36 passed (31 pre-existing + 5 new).
- `cargo test -p smelt-cli --features databricks --test github_activity_databricks --test dbx_dogfood_provision --quiet` — 9 + 4 passed.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, shellcheck, full workspace `cargo test`, `example_diagnostics`). No ratchet lowered.
- Live: full-refresh run `20260912-103014-1e03d5` — 11 success / 1 failed / 4 skipped, described above.
