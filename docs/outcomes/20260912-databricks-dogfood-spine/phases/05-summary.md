# Phase 5 summary — first live load: two fixture days into Unity Catalog

## Shipped

- `python/smelt/databricks_adapter.py`: `load_arrow_table` takes an optional
  `mode="overwrite"` kwarg. `"append"` skips the `tableExists`/`DROP TABLE`
  probe and writes `df.write.mode("append").saveAsTable(...)`; the default
  keeps today's drop-and-recreate behaviour, so the Rust call site
  (`crates/smelt-backend-spark/src/lib.rs:700`, two positional args) is
  unchanged.
- Same file: `createDataFrame` now receives `table.to_pandas()`, not the raw
  `pyarrow.Table` — see Decisions.
- `scripts/dbx-dogfood-loader.py`: `cmd_execute` passes `mode="append"` for
  both tables; the stale phase-5 NOTE is gone. New `--apply-ddl` mode executes
  the same statements `--emit-ddl` prints, both sourced from one
  `ddl_statements()` list. `duckdb_query_arrow` now `LOAD arrow;`s before the
  `COPY ... FORMAT arrow` — see Decisions.
- `crates/smelt-cli/tests/dbx_dogfood_loader.rs`: 5 new tests (13 total, all
  green) — `load_arrow_table_appends_without_dropping`,
  `load_arrow_table_default_mode_still_replaces` (both gated on
  `.smelt-dbx-venv` having `pyarrow`, skip otherwise),
  `loader_execute_path_appends_rather_than_replacing`,
  `apply_ddl_executes_exactly_the_emitted_ddl`, `apply_ddl_needs_no_network_to_emit`.
- `crates/smelt-cli/tests/dbx_dogfood_provision.rs`: `dbx-auth.sh` moved from
  `SECRET_SCRIPTS` to `READ_ONLY_SCRIPTS` to match the settings.json change
  already on disk resolving `## Blocked` item (b) — see Decisions.
- **Live**: `workspace.smelt_dogfood.github_events` and `.github_events_arrival`
  hold 2026-08-05 and 2026-08-06 (5,978 rows each table; 3,201 / 2,777 per
  `ingested_date`), the `_loader_days` ledger has both dates, and the
  redelivered slice (`created_at` 2026-08-05, `ingested_date` 2026-08-06) is
  exactly 63 rows — every number matches the DuckDB-computed expectation
  exactly. Re-running `--date 2026-08-06` was a no-op ("already loaded,
  skipping") with unchanged counts.

## Decisions

- **`table.to_pandas()` before `createDataFrame`, not a pyarrow-native path.**
  The live load failed on day 1 with `CANNOT_INFER_TYPE_FOR_FIELD` — Databricks
  Connect's `pyspark.sql.connect.session.SparkSession.createDataFrame` (pyspark
  3.5.0) has no `pyarrow.Table` overload; passing one directly makes it treat
  the table as row-iterable data. This is the load-cannot-complete-at-all case
  the plan calls out as the one exception to "record, don't fix" — fixed, with
  a regression test asserting `createDataFrame` receives a
  `pandas.DataFrame` (not `pandas.core.frame.DataFrame` — pyspark 3.5.0
  reports the class as `pandas.DataFrame`).
- **`LOAD arrow;` prefix in `duckdb_query_arrow`.** The local `duckdb` CLI
  (v1.4.4) has `arrow` as a community extension, not an autoload-known one —
  `COPY ... FORMAT arrow` fails cold with "Copy Function with name arrow does
  not exist" even after a one-time `INSTALL ... FROM community` because that
  only caches the binary, it doesn't make DuckDB autoload it. An explicit
  `LOAD arrow;` in the same `-c` statement is now always run; the one-time
  `INSTALL FROM community` step is left to whoever sets up the machine (noted
  in the function's docstring) since it needs network once, matching how
  `mise run setup-duckdb` already handles the core DuckDB library.
- **`dbx-auth.sh` test/settings drift resolved, not re-litigated.** Found
  `.claude/settings.json` and `outcome.md`'s item (b) already edited on disk
  (uncommitted) to move `dbx-auth.sh` deny→allow — it only ever prints an
  expiry stamp, never the token, matching the outcome's own stated rationale.
  This left `dbx_dogfood_provision.rs`'s `secret_scripts_are_denied...` test
  red against a decision already made elsewhere; updated the test's
  `SECRET_SCRIPTS`/`READ_ONLY_SCRIPTS` split to match rather than reverting
  the settings change or leaving the gate red.

## For the next planner

- The gpg-agent cache-TTL follow-up in `## Blocked` item (b) (raising
  `default-cache-ttl`/`max-cache-ttl` in `~/.gnupg/gpg-agent.conf`) is still
  open and still human-only — `dbx-auth.sh` itself failed here with "problem
  with the agent: Inappropriate ioctl for device" (no TTY for a passphrase
  prompt in this headless run). This run's live steps only needed
  `dbx-query.sh`/the loader against an *already-minted* token, so it never hit
  this — phases 6–9 (full refresh, 3+ incremental windows) run longer and are
  the first phases that will.
- Free Edition surprise for the facts sheet: `INVALID_HANDLE.SESSION_CLOSED`
  fired again on `execute_sql_no_result`/`execute_sql` calls made shortly after
  a prior session's `adapter.close()` — consistent with phase 4c's measured
  "single-digit-second serverless teardown" note, not a new defect. It is a
  `UserWarning`, not a failure; every call still returned correct data.
- `crates/smelt-cli/tests/dbx_dogfood_loader.rs` now has two tests
  (`load_arrow_table_appends_without_dropping`,
  `load_arrow_table_default_mode_still_replaces`) that only run when
  `.smelt-dbx-venv` has `pyarrow` importable. They ran and passed in this
  worktree; a fresh checkout without that venv built will skip them silently
  (`eprintln!` + return), matching the `SPARK_CONNECT_URL` gating convention —
  flagged in case a future CI wiring wants these to be hard-required instead.
- Out of scope, correctly: `python/smelt/spark_adapter.py` untouched.

## Gates

- `cargo test -p smelt-cli --test dbx_dogfood_loader --quiet` — 13 passed.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both
  feature sets, shellcheck, full workspace `cargo test`, example_diagnostics).
- Live: task-8 counts match the fixture exactly (events 5,978; arrival 5,978;
  per-day 3,201/2,777; redelivered slice 63); task-7 re-run of 2026-08-06 left
  counts unchanged and reported "already loaded, skipping".
