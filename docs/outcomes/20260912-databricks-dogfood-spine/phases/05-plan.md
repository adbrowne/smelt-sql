# Phase 5 plan — first live load: two fixture days into Unity Catalog

## Objective

Close success criterion 5: land at least two fixture days in
`workspace.smelt_dogfood` through `scripts/dbx-dogfood-loader.sh`, with
`ingested_date` stamped and the redelivered slice present, verified by row
counts read back from Unity Catalog against the Parquet fixture's own counts.
The blocking defect phase 3 recorded — `DatabricksAdapter.load_arrow_table`
drops and recreates its target table, so a second day would erase the first —
is fixed first, offline and red-green, before any live write. This is also the
first real exercise of the Arrow load path that criterion 3 specifies, so it
gates phases 6–9.

## Spec delta

None. The loader is external to smelt and the adapter change is an
implementation detail behind an unchanged Rust-side call
(`crates/smelt-backend-spark/src/lib.rs:700` passes two positional arguments);
`docs/specs/multi_backend.md` §"Loading data into a backend" already states the
rule this phase obeys (rows cross as Arrow, never a host-visible file).

## Tests

Offline, all in `crates/smelt-cli/tests/dbx_dogfood_loader.rs` unless noted:

1. `load_arrow_table_appends_without_dropping` — drive
   `python/smelt/databricks_adapter.py` with a recording fake `spark` object
   (construct via `object.__new__`, so no `databricks-connect` import and no
   network): `mode="append"` issues **no** `DROP TABLE` and writes with
   `.mode("append")`.
2. `load_arrow_table_default_mode_still_replaces` — the same fake with no
   `mode` argument keeps today's drop-and-recreate behaviour, so
   `SparkBackend`'s existing two-positional-argument call site is unchanged.
3. `loader_execute_path_appends_rather_than_replacing` — the loader's execute
   path passes append mode for both the events and the arrival table; a
   replace-mode load of a second day would be a silent history loss.
4. `apply_ddl_executes_exactly_the_emitted_ddl` — `--apply-ddl` runs the same
   statements `--emit-ddl` prints, parsed from the one emitter rather than
   restated (same no-drift discipline as `parsed_modulus`), asserted with a
   recording fake adapter and no workspace.
5. `apply_ddl_needs_no_network_to_emit` — `--emit-ddl` still touches no network
   and needs no credential (regression guard on the split).

## Tasks

1. Add an optional `mode="overwrite"` keyword to
   `DatabricksAdapter.load_arrow_table`; `"append"` skips the `DROP TABLE` and
   writes `df.write.mode("append").saveAsTable(...)`. Leave
   `python/smelt/spark_adapter.py` untouched — the Spark parity tier is not in
   this phase's blast radius.
2. Make the loader's `cmd_execute` pass `mode="append"` for both tables and
   delete the stale phase-5 NOTE comment it carries.
3. Add a `--apply-ddl` mode to `scripts/dbx-dogfood-loader.py` that executes
   the statements `cmd_emit_ddl` produces (refactor the emitter to return a
   list of statements that both modes consume) via
   `execute_sql_no_result`.
4. Write tests 1–5 red first, then make them green.
5. **Live**: confirm reachability with
   `bash scripts/dbx-query.sh "SELECT current_user() AS u"`. If the workspace
   is unreachable or the OAuth token has expired (outcome `## Blocked` item
   (b)), stop and emit `<<PHASE_BLOCKED>>` — never skip green, never mint a
   token.
6. **Live**: `bash scripts/dbx-dogfood-loader.sh --apply-ddl`, then
   `--date 2026-08-05` and `--date 2026-08-06` (the fixture's first two days;
   day 1 has an empty redelivery arm by construction, day 2 carries the
   redelivered slice, so the pair proves both arms).
7. **Live**: re-run `--date 2026-08-06` and confirm it is a no-op
   ("already loaded, skipping") with row counts unchanged — per-day idempotence
   against the real Unity Catalog ledger, not the dry-run store.
8. **Live**: read counts back with `scripts/dbx-query.sh` and compare to the
   fixture's own numbers computed with `duckdb`:
   `github_events` total, `github_events_arrival` total, per-`ingested_date`
   counts, and the redelivered slice count for 2026-08-06
   (`created_at::date = 2026-08-05 AND ingested_date = 2026-08-06`, expected
   `1/modulus` of day 1). Record every number verbatim in the summary.
9. Record in the summary any Free Edition surprise the live load surfaced
   (Arrow type coercion, `saveAsTable` schema resolution, session teardown
   warnings) — recorded, not fixed, unless the load cannot complete at all.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test dbx_dogfood_loader --quiet 2>&1 | tail -20`
- Live: the counts of task 8 agree with the fixture exactly; the task-7 re-run
  leaves them unchanged.

## Commit message

`outcome(databricks-dogfood-spine): phase 5 lands two fixture days on Databricks`
