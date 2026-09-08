# Phase 09 summary — the loader artifact, with no cloud

**Shipped:**
- `scripts/bq-dogfood-loader.sh`: `--emit-ddl`, `--emit-sql --date <YYYY-MM-DD>`, and a
  stubbed default (execute) mode that refuses with a clear "phase 10" message. Derives its
  load SQL from `examples/github_activity/sample.sql` by slicing from the first bare
  `SELECT` line to EOF and substituting only the `_TABLE_SUFFIX BETWEEN` line's values —
  every other line passes through byte-identical.
- The redelivery arm (`MOD(CAST(id AS BIGINT), 50) = 0` over day D-1) is layered as an
  outer `WHERE` around the untouched base query wrapped in a subquery, so the base query's
  own lines are never touched by the arm split either.
- Both target tables in one script: `raw.github_events` (no `ingested_date`) and
  `raw.github_events_arrival` (`SELECT *, DATE '<D>' AS ingested_date FROM (<base>) WHERE
  ...`), mirroring `setup_sources.sql`/`run_incremental.py`'s existing DuckDB-side pattern.
- `--emit-ddl` prints both tables' DDL: `PARTITION BY DATE(created_at)` /
  `PARTITION BY ingested_date`, `CLUSTER BY repo_id`, `partition_expiration_days` parsed
  from `README.md` (not hard-coded twice), no table-level expiry set.
- A year-boundary `--date` (D-1 in a different year than D) is refused with a
  `year`-mentioning stderr message rather than silently emitting a broken suffix range.
- `crates/smelt-cli/tests/github_activity_loader.rs`, 7 tests (6 required + the
  `shellcheck`-gated one, which skips — `shellcheck` isn't on this box).
- `examples/github_activity/README.md`: new "The BigQuery loader" section documenting the
  script, both invocation modes, and the retention bound (45 days: 30-day fixture range
  plus backfill headroom) in the one place the DDL parses it from.

**Decisions:**
- The base query is wrapped as a subquery (`SELECT * FROM (<base>) WHERE <arm filter>`)
  rather than appending the arm filter as another `AND` line inside the base query's own
  `WHERE` clause — keeps the "only one line differs from sample.sql" property exact and
  mechanically checkable, rather than approximate.
- `ingested_date` is added via an outer `SELECT *, DATE '<D>' AS ingested_date FROM (...)`
  wrapper rather than folded into the base SELECT list, for the same reason — the base
  query's SELECT list must stay byte-identical to `sample.sql`'s.
- Retention set to 45 days per the plan's own reasoning (30-day fixture + backfill
  headroom); see the outcome's decision log for the resulting conflict with a
  pre-existing, unrelated `retention: '90 days'` field on the two source YAMLs (inert
  today, not touched here).

**For the next planner:**
- `models/sources/raw/github_events{,_arrival}.yml`'s `retention: '90 days'` comment
  ("matches the loader's own N-day trim") is now stale — the loader declares 45. That
  field is unconsumed by any maintenance logic today (verified by grep), so nothing broke,
  but whoever next touches those files for `20260906-trimmed-history-sources` should
  reconcile the number or correct the comment.
- Phase 10 (deploy + measure cost per run) is still fully human-gated — no GCP identity
  exists in this worktree (`gcloud auth list` → no credentialed accounts, unchanged from
  phase 7/8's findings).

**Gates:**
- `cargo test -p smelt-cli --test github_activity_loader` — 7 passed.
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets,
  workspace tests, example_diagnostics).
- `bash .claude/scripts/large-file-check.sh` — OK.
- Manual: `bash scripts/bq-dogfood-loader.sh --emit-sql --date 2026-08-06` diffed
  line-for-line against `sample.sql` — only the `_TABLE_SUFFIX` line differs.
