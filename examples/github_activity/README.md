# github_activity

A GitHub-events pipeline that runs on **both** DuckDB and BigQuery over the same rows.
DuckDB is not a fallback here: it is the cheap oracle that makes a dual-target diff the
least expensive way to find defects the offline gates cannot see.

Outcome: `docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md`.

## The sample

`sample.sql` is the contract. It selects a stable 0.1% slice of GitHub Archive —
`MOD(repo.id, 1000) = 0`, 30 days, `payload` not projected — and **both legs must see
exactly these rows**: the DuckDB leg reads the Parquet export of it, and the BigQuery
loader reproduces the same query into `raw.github_events`. Two targets over different
populations are not comparable, so the parity check would be measuring nothing.

`seeds/github_events_sample.parquet` is that export, committed so the DuckDB leg runs in
ordinary CI with no warehouse and no credentials. Regenerate it with:

```bash
bash scripts/bigquery-auth.sh                     # mint a 1h token
bash examples/github_activity/refresh_sample.sh   # ~6.3 GB scanned, about US$0.03
```

At the pinned range that is 64,313 events over 2026-08-05 … 2026-09-03: 4,491 actors,
5,016 repos, 13 event types, 34 repos observed under more than one name.

Three properties of the data worth knowing before reading any output:

- **The upstream feed contains no duplicate event ids.** Deduplication is needed because
  the *loader* is declared at-least-once, not because GitHub Archive repeats itself
  (`raw.github_events` measured at 64,313 rows, 64,313 distinct ids). So the loader
  deliberately redelivers on purpose: each day's load re-appends a deterministic 2% slice
  of the *previous* day's rows (`MOD(CAST(id AS BIGINT), 50) = 0`), byte-identical to the
  original including `created_at`. `raw.github_events` therefore permanently contains
  synthetic duplicates, and `bronze.events`, being a passthrough, carries them — this is
  correct, not a bug, and is what makes the bronze→silver boundary mean anything. A run
  that never replays a day never exercises `silver.events_deduped`.
  (`docs/outcomes/20260906-bigquery-dogfood-spine/phases/02-plan.md` §"Redelivery is not
  free".)
- **The sample skews to newly-created repositories.** `repo.id` is uniform over ids, and
  recent ids are dominated by bulk repo creation, so 92% of events are `PushEvent` and the
  median repo has a single event. Sessionization and dedup are unaffected; anything
  shaped like a leaderboard will look odd, and that is the sample, not a bug.
- **`repo.id` rather than a `repo.name` prefix is what makes the sample stable**: a name
  prefix would silently drop a repository the moment it was renamed, corrupting exactly
  the rename history this pipeline exists to model.

## The DuckDB leg

`examples/github_activity/` runs four models against DuckDB with no warehouse and no
credentials: `bronze.events` (a typed passthrough of the source), `silver.events_deduped`
(dedup on `id`), `silver.actor_sessions` (30-minute-gap sessionization per actor, ported
from `examples/web_analytics/functions/sessionize.sql`), and
`marts.daily_active_contributors`. `setup_sources.sql` creates the empty
`raw.github_events` table; `run_incremental.py` replays the fixture day by day, redelivering
the previous day's slice as described above, and finishes with `smelt test`:

```bash
python3 examples/github_activity/run_incremental.py
```

`silver.events_deduped` is `grain: key` with `timeseries: { partition_column:
first_seen_date }` — the composed key-addressed-and-time-partitioned shape
(`docs/specs/incremental_shapes.md` §"Key temporal locality"). Locality is established via
**route 3 (recurrence-bounded)**: `raw.github_events` declares
`mutation_profile.key_recurrence: { key: [id], window: '0 days' }` (a genuine redelivered
duplicate always shares its original's `created_at` exactly), checked at merge time
— a duplicate pair that violates it fails the run transactionally
(`KeyedRecurrenceBoundViolated`) rather than silently mis-dedupping, which
`crates/smelt-cli/tests/github_activity_replay.rs` exercises as a negative control. Unlike
`examples/web_analytics/silver/events_parsed.sql`'s arrival-based lateness filter, there is
no independent ingestion-time column here to derive a WHERE-clause lookback from — one
arrives with a loader-stamped `ingested_date` in a later phase — so the model's
`maintenance.scan_bounds` declares `allow_full_scan: true` on `raw.github_events` instead;
the keyed `MERGE` this model compiles to is idempotent regardless of window width, so the
full scan costs re-read, never correctness.

## The rename stream: two succession models, two partition postures

`silver.repo_naming` and `silver.actor_naming` are recognised as the succession grain
(`docs/specs/incremental_shapes.md` §"The succession grain") from their SQL shape alone —
neither declares `grain:`, `unique_key:` or `timeseries:`. Each is one row per
`(key, created_at)` carrying the name in force at that event, not one row per rename:
"only the rows where the name changed" needs `LAG`, and the classifier admits exactly one
row-local pre-window filter, so that reduction happens downstream in
`marts.naming_history` instead, which unions both histories and keeps the rows where
`LAG(name)` differs from the current name.

The two models exercise both succession partition postures from one pipeline
(`docs/outcomes/20260906-bigquery-dogfood-spine/phases/03-plan.md`):

- `silver.repo_naming` reads `raw.github_events` directly — **event-time-partitioned**
  (`timeseries.partition_column == event_time_column == created_at`). The loader's
  deliberate previous-day redelivery lands in a **closed** partition here, exercising the
  append-only probe's late-arrival classification.
- `silver.actor_naming` reads `raw.github_events_arrival` — a second physical relation,
  same columns plus a loader-stamped `ingested_date`, **arrival-partitioned**
  (`partition_column: ingested_date` differs from `event_time_column: created_at`). The
  same redelivered rows are stamped with *today's* `ingested_date` there, landing in the
  **open** partition instead.

Both `created_at` clock columns are projected verbatim (not aliased away): the
succession-patch technique's tombstone ledger resolves the clock column's type from the
model's own output schema by name, so the clock column must survive under its source name.

**A genuine divergence, discovered rather than fixed here**: `silver.repo_naming` and
`silver.actor_naming` do not satisfy the full-refresh/incremental equivalence invariant
(criterion 7) at the raw row-count level. The window-forward patch loop addresses the
presented table by `(key, clock)` (its `MERGE ... ON` condition), so a redelivered
duplicate or a same-second tie whose payload agrees converges to one presented row;
`--full-refresh` re-runs the model's raw compiled `SELECT` with no such addressing, so it
keeps every tied row. The gap matches exactly the fixture's own measured tie counts (139
extra rows for `repo_naming`, 145 for `actor_naming`) —
`crates/smelt-cli/tests/github_activity_replay.rs`'s
`full_refresh_matches_incremental_replay` asserts the divergence explicitly rather than
silently tolerating it. `marts.naming_history` is unaffected, since its `LAG`-based
"only where the name changed" filter drops a duplicated tie row identically on both legs.
This is recorded for `docs/outcomes/20260906-scd2-keyed-succession`'s decision log, not
fixed in this pipeline.

## Gold and marts

`gold.repo_dim` (one row per repo, current name from `silver.repo_naming`'s `is_current`
flag), `gold.events_enriched` (every deduped event enriched with the repo's current name —
the `LEFT JOIN`-against-a-`unique_key`-declaring-dimension shape), `gold.repo_activity_daily`
(per-`(repo, day)` event and distinct-actor counts), and two marts over it:
`marts.repo_leaderboard` (total events per repo — reproduces the sample's documented skew
rather than hiding it: the top row is a single bot repo with 1,750 of the fixture's 64,313
events) and `marts.star_growth` (a cumulative daily `WatchEvent` count, thin on purpose — the
fixture holds only 47 `WatchEvent`s over 30 days).

**A genuine derivation gap, discovered and recorded rather than fixed here**: no maintenance
cell is ever derived for `gold.repo_dim`'s mutation sensitivity — a repo rename does not
re-derive `gold.events_enriched.current_repo_name` on already-written rows through any
tracked technique. `gold.repo_dim` is a clockless upstream model (no `timeseries:`), and
`gold.events_enriched` is `grain: partition`; the only route open to a clockless
upstream (`append_model_edge_cells`'s key-addressed route) needs the *downstream's own*
declared `unique_key` to scope the recompute, and a `grain: partition` output has none by
construction. `smelt explain gold.events_enriched --json` shows the resulting
`RepairKeysNotDiscoverable` refusal rather than an `UpstreamMutation(gold.repo_dim)` cell —
characterised by `events_enriched_dimension_mutation_cell_technique` in
`crates/smelt-cli/tests/github_activity_replay.rs`. This is a criterion-8 finding for
`docs/outcomes/20260906-bigquery-correctness`, not fixed in this pipeline.

The typed silver fan-out (`push_events`, `pr_events`, `issue_events`, `star_events`) needs
`payload`, which needs a `sample.sql` re-pin and a human-minted BigQuery token — it is a
separate, currently blocked phase (`docs/outcomes/20260906-bigquery-dogfood-spine/outcome.md`
phase 5), not an oversight.
