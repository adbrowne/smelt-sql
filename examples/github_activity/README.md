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

Two properties of the data worth knowing before reading any output:

- **The upstream feed contains no duplicate event ids.** Deduplication is needed because
  the *loader* replays overlapping windows, not because GitHub Archive repeats itself. A
  test that never replays a window never exercises the dedup.
- **The sample skews to newly-created repositories.** `repo.id` is uniform over ids, and
  recent ids are dominated by bulk repo creation, so 92% of events are `PushEvent` and the
  median repo has a single event. Sessionization and dedup are unaffected; anything
  shaped like a leaderboard will look odd, and that is the sample, not a bug.

`repo.id` rather than a `repo.name` prefix is what makes the sample stable: a name prefix
would silently drop a repository the moment it was renamed, corrupting exactly the rename
history this pipeline exists to model.
