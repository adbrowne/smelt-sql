# Phase 09 — Author the loader artifact, with no cloud

**Outcome:** `docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md`
**Advances:** criterion 3 (loader reproduces `sample.sql` **verbatim**, day-partitioned,
day-range-bounded, N-day trimmed), and through it criterion 6 (two targets over the *same*
population), criterion 9 (both partition postures reachable on BigQuery too), criterion 10.

## Objective

Write the dogfood loader as a committed, testable artifact that needs no GCP project to
verify: one script that *derives* its load SQL from `examples/github_activity/sample.sql`
rather than restating it, plus the `raw.github_events` DDL and the retention bound. A per-PR
test drives the script's `--emit-sql` mode and asserts the projection, the
`FROM \`githubarchive.day.2026*\`` wildcard and the `MOD(repo.id, 1000) = 0` filter arrive
byte-identical, with only the `_TABLE_SUFFIX` range parameterised — so "verbatim" is a gate,
not a hope. Deploying it and measuring cost per run is phase 10 (human-gated).

## Spec delta

None. The loader is external to smelt by this outcome's own §"Out of scope" (`produced_by:`
belongs to `20260906-external-dag-steps`, the retention bound to
`20260906-trimmed-history-sources`); nothing user-visible in smelt changes. The retention
bound is recorded in prose, in `examples/github_activity/README.md`.

## Tests

New binary `crates/smelt-cli/tests/github_activity_loader.rs`, all driving
`scripts/bq-dogfood-loader.sh --emit-sql --date <D>` with no network and no `bq` on PATH:

1. `loader_reproduces_the_sample_projection_and_filter_verbatim` — the emitted SQL contains
   `sample.sql`'s SELECT list, its `FROM` wildcard and its `MOD(repo.id, 1000) = 0` line as
   contiguous byte-identical text; the *only* line of `sample.sql`'s body that differs is the
   `_TABLE_SUFFIX BETWEEN` one. Compare line-wise against the real `sample.sql` read from
   disk, so a future re-pin (phase 5's `payload` column) fails this test rather than drifting.
2. `loader_suffix_range_is_bounded_to_the_requested_days` — `--date 2026-08-06` emits
   `_TABLE_SUFFIX BETWEEN '0805' AND '0806'` (day D plus D-1 for the redelivery arm) and no
   unbounded `day.*`; a `--date` spanning a year boundary is refused with a clear message.
3. `loader_redelivery_matches_the_duckdb_replay_driver` — the redelivery arm's predicate is
   `MOD(CAST(id AS BIGINT), 50) = 0` over day D-1, and `50` is asserted equal to
   `REDELIVERY_MODULUS` parsed out of `examples/github_activity/run_incremental.py`, so the
   two legs cannot drift apart silently.
4. `loader_stamps_ingested_date_on_the_arrival_table_only` — the emitted script writes both
   `raw.github_events` (no `ingested_date`) and `raw.github_events_arrival` (`ingested_date`
   = the run day for real *and* redelivered rows), mirroring `setup_sources.sql`.
5. `ddl_declares_day_partitioning_and_the_documented_retention_bound` — `--emit-ddl` yields
   `PARTITION BY DATE(created_at)` for the event-time table, `PARTITION BY ingested_date` for
   the arrival twin, `partition_expiration_days` equal to the N documented in `README.md`
   (parsed from the README, not hard-coded twice), and sets no *table* expiry (criterion 1).
6. `emit_sql_touches_no_cloud` — run with `PATH` stripped of `bq`/`gcloud` and
   `SMELT_BQ_ACCESS_TOKEN` unset; the script still exits 0 and prints SQL.
7. `loader_script_is_shellcheck_clean` — only if `shellcheck` is on PATH, else skipped.

## Tasks

1. Red: add `github_activity_loader.rs` with tests 1–6 against a not-yet-existing script.
2. Write `scripts/bq-dogfood-loader.sh`: modes `--emit-ddl`, `--emit-sql --date <YYYY-MM-DD>`,
   and (default) execute-via-`bq`. It reads `examples/github_activity/sample.sql`, splices out
   the `_TABLE_SUFFIX BETWEEN …` line and substitutes the computed `MMDD` range, and wraps the
   result in the two `INSERT`s (real day D; redelivered 2% slice of D-1) for both tables. It
   never restates the projection or the sample filter.
3. Add the `raw.github_events` / `raw.github_events_arrival` DDL to the same script's
   `--emit-ddl`: day-partitioned, clustered on `repo_id`, `partition_expiration_days = 45`,
   dataset default table expiry untouched.
4. Green tests 1–6; add test 7.
5. `README.md`: a "The BigQuery loader" section — the script is **external to smelt** and
   at-least-once by construction, the retention bound is 45 days (why: 30-day fixture range
   plus headroom for a late backfill), how a human deploys it (defer detail to phase 10), and
   a `cost per run: measured in phase 10` placeholder line.
6. Note in `scripts/bq-dogfood-loader.sh`'s header that phase 10 turns the emitted SQL into a
   scheduled query; nothing here schedules anything.

## Verification

- `bash .claude/scripts/verify-phase.sh`
- `cargo test -p smelt-cli --test github_activity_loader`
- `bash scripts/bq-dogfood-loader.sh --emit-sql --date 2026-08-06` reviewed by eye once, and
  the emitted text diffed against `sample.sql` to confirm the only delta is the suffix range.
- `bash .claude/scripts/large-file-check.sh`

## Commit message

`feat(github_activity): derive the BigQuery dogfood loader from sample.sql, gated verbatim`
