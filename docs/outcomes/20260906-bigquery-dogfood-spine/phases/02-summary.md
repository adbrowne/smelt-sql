# Phase 2 summary — `examples/github_activity/` green on DuckDB

**Shipped:**
- `examples/github_activity/{smelt.yml, setup_sources.sql, run_incremental.py, .gitignore}`.
- Source `models/sources/raw/github_events.yml`: `mutation_profile` (append_only, `lateness:
  '6 hours'`, `redelivery: at_least_once`, `key_recurrence: {key: [id], window: '0 days'}`),
  `unique_key: [id]`, `retention: '90 days'`, `timeseries` on `created_at` (event_time and
  partition column both `created_at`, matching `examples/broken/models/sources/
  succession_changes.yml`'s precedent for a raw-timestamp partition column).
- Four models: `bronze.events` (passthrough), `silver.events_deduped` (`grain: key`,
  `timeseries` on `first_seen_date`), `silver.actor_sessions` (`grain: partition`, 30-min
  gap sessionization via a ported `functions/sessionize.sql`), `marts.
  daily_active_contributors`.
- `crates/smelt-cli/tests/github_activity_replay.rs` — 4 tests (redelivery/dedup count,
  recurrence-bound violation, midnight-session, full-refresh equivalence over all 30 days).
  Wired into per-PR CI: `example_diagnostics/smoke_and_migration.rs` +
  `smelt-lsp/tests/example_workspaces.rs` + `e2e/example_builds.rs`'s KNOWN_UNBUILDABLE.
- README corrected: previous-day redelivery replaces the superseded overlapping-window text.

**Decisions:**
- **`silver.events_deduped`'s "two-day lookback" is `allow_full_scan: true` + declared
  `key_recurrence`, not a Form-B WHERE filter — a deviation from the plan's literal task 6
  wording.** Why: a Form-B lookback (`events_parsed`'s style) requires a WHERE clause
  relating the model's output partition column to a *different* source column that
  correlates with when a row became visible (`events_parsed` uses `arrival_time`).
  `raw.github_events` in this phase has no such column — the redelivered duplicate is
  byte-identical to the original, including `created_at` — so any self-referential filter
  on `created_at` is tautological (zero margin, the "transparent slice" case) and cannot
  widen anything. Empirically verified both ways: removing `allow_full_scan` still produces
  correct dedup counts in the replay (a keyed `MERGE` is idempotent regardless of window
  width — reprocessing an already-correct id is a no-op), so there is no "narrow the
  lookback and watch duplicates survive" failure mode to construct for this design, unlike
  `events_parsed`'s. The negative control that *does* exist and *is* tested: a duplicate
  pair that violates the declared zero-width `key_recurrence` fails the run transactionally
  (`recurrence_bound_violation_fails_the_run`), proving the declared bound is checked, not
  decorative. This mirrors `examples/web_analytics/silver/events_deduped.sql`'s own
  precedent exactly (same route 3, same `allow_full_scan` escape hatch, same reason).
- `mutation_profile.lateness: '6 hours'` is a placeholder, not a real measurement. Task 2
  asked to measure the fixture's shard-lag against the day it landed in, but the committed
  Parquet fixture carries no ingestion-time column (that arrives with `sample.sql`'s live
  BigQuery source, not the export) — there is nothing in the fixture to measure this from
  offline. `lateness` is orchestration-only (`docs/specs/sources.md` §Semantics: "never a
  plan input... cannot affect correctness"), so the placeholder cannot break anything; a
  live measurement is deferred to whenever a session has BigQuery access again.
- Reused `examples/web_analytics/functions/sessionize.sql` verbatim (ported, per project
  isolation) rather than writing new sessionization SQL; `partition_col` widened from
  `Expr<Integer>` to `Expr<BigInt>` since `actor_id` is `BIGINT`. `platform_col` is passed a
  constant `NULL` literal (no platform axis in GitHub activity), which permanently disables
  that boundary arm without changing the function's shape.
- `marts.daily_active_contributors` reads only from `silver.actor_sessions` (not also from
  `silver.events_deduped`) to avoid a two-clocked-source join on a partition-grain output;
  `total_events` is `SUM(event_count)` from the session rows instead.

**For the next planner:**
- Phase 3 (succession) needs `ingested_date` per the outcome doc's own decision log — this
  phase confirms *why* it's needed structurally: it's the only way to give a future
  Form-B-style model an independent ingestion axis distinct from event time, which this
  phase's redelivery mechanism could not use.
- The measured lateness value (6h placeholder) should be revisited once real BigQuery
  shard-lag data is available (phase 7+); low priority since it's orchestration-only.
- `retention: '90 days'` is a guess pending phase 7's actual loader trim value; revisit then.
- Not built here (deliberately, per the plan): campaign-style attribution, `payload`
  extraction, `gold.events_enriched`. Phase 4 adds these.

**Gates:**
- `bash .claude/scripts/verify-phase.sh` — ALL GREEN (fmt, clippy both feature sets, full
  `cargo test` workspace, `example_diagnostics`).
- `cargo test -p smelt-lsp --test example_workspaces github_activity` — 1 passed.
- `cargo test -p smelt-cli --test github_activity_replay` — 4 passed (16.2s).
- `python3 run_incremental.py` (manual, 30-day full replay): 65,583 raw rows (64,313 real +
  1,270 redelivered) dedup to exactly 64,313 distinct ids; `smelt test` clean.
