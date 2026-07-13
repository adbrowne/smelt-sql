<!-- GENERATED FILE — edit examples/web_analytics/tutorial_pages/first-model.md and run python3 examples/web_analytics/generate_tutorial.py -->

# A first incremental model

The pipeline starts as small as possible: one source, one typed
passthrough, and one incremental model that parses events into columns.
No dedup, no lateness handling yet — this page is about the mental model
you'll reuse everywhere else: **partitions, run windows, and reading the
exact SQL a run will execute.**

This stage is a complete project at
[`tutorial_stages/01_first_model/`](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics/tutorial_stages/01_first_model).

## The source and the bronze layer

The feed lands as a raw table with string-typed timestamps and a JSON
payload. A source declaration
([`models/sources/raw/events.yml`](https://github.com/adbrowne/smelt-sql/blob/main/examples/web_analytics/tutorial_stages/01_first_model/models/sources/raw/events.yml))
names it and pins its columns, and a one-line bronze model casts the
clocks and the partition value to real types so everything downstream is
typed. (If you're coming from dbt: sources and staging models, same idea.)

## The model

`silver.events_parsed`, first version — a plain SELECT plus a frontmatter
block:

<!-- smelt-include: tutorial_stages/01_first_model/models/silver/events_parsed.sql -->
```sql
---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
---
SELECT
    event_id,
    device_id,
    user_id,
    CASE WHEN user_id IS NOT NULL
         THEN 'u:' || CAST(user_id AS VARCHAR)
         ELSE 'd:' || CAST(device_id AS VARCHAR)
    END AS amplitude_id,
    CAST(event_time AS TIMESTAMP) AS event_ts,
    CAST(event_date AS DATE) AS event_date,
    utm_campaign,
    json_extract_string(payload, '$.event_name') AS event_name,
    json_extract_string(payload, '$.platform') AS platform,
    json_extract_string(payload, '$.url') AS url
FROM smelt.bronze.raw_events
```

The SQL is ordinary: cast the clocks, pull three payload fields out of the
JSON, and synthesise `amplitude_id`, a never-NULL identifier that prefers
the signed-in `user_id` and falls back to the device (the [identity
example](../web_analytics.md) builds on it; here it's just a column).

The frontmatter is where smelt learns how this table lives in time:

- `refresh: incremental` with `grain: partition` — maintain this table in
  partition-sized pieces rather than rebuilding it whole.
- The `timeseries:` block names the clock: every row belongs to a day, by
  `event_date`. **A day of this table is the unit smelt reads, writes, and
  reasons about.** (Full key reference:
  [timeseries](../../reference/timeseries.md); the narrative version is in
  the [incremental models guide](../../guide/incremental-models.md#configuration).)

## Running it

```bash
# First build: load sources' dependents and materialize everything
smelt build

# A daily run: process one day's window
smelt run --event-time-start 2026-04-10 --event-time-end 2026-04-11
```

The `--event-time-start`/`--event-time-end` pair is the **run window** —
the slice of time you're asking smelt to bring up to date. Windows are
half-open (`[start, end)`), and nothing requires them to be one day: a
30-day window is one run.

## What actually runs

Before trusting any of this, look at it. `smelt explain` prints the exact
maintenance statements a run over a given window would execute:

```bash
smelt explain silver.events_parsed --show-sql --period 2026-04-10..2026-04-11
```

<!-- smelt-generate: @cwd=tutorial_stages/01_first_model explain silver.events_parsed --show-sql --json --period 2026-04-10..2026-04-11 -->
```sql
-- trigger: Backfill
BEGIN
  DELETE FROM main.silver_events_parsed WHERE event_date >= '2026-04-10' AND event_date < '2026-04-11'
  INSERT INTO main.silver_events_parsed SELECT CAST(event_id AS BIGINT) AS event_id, CAST(device_id AS INTEGER) AS device_id, CAST(user_id AS INTEGER) AS user_id, CAST(amplitude_id AS VARCHAR) AS amplitude_id, CAST(event_ts AS TIMESTAMP) AS event_ts, CAST(event_date AS DATE) AS event_date, CAST(utm_campaign AS VARCHAR) AS utm_campaign, CAST(event_name AS VARCHAR) AS event_name, CAST(platform AS VARCHAR) AS platform, CAST(url AS VARCHAR) AS url FROM (
  SELECT
      event_id,
      device_id,
      user_id,
      CASE WHEN user_id IS NOT NULL
           THEN 'u:' || CAST(user_id AS VARCHAR)
           ELSE 'd:' || CAST(device_id AS VARCHAR)
      END AS amplitude_id,
      CAST(event_time AS TIMESTAMP) AS event_ts,
      CAST(event_date AS DATE) AS event_date,
      utm_campaign,
      json_extract_string(payload, '$.event_name') AS event_name,
      json_extract_string(payload, '$.platform') AS platform,
      json_extract_string(payload, '$.url') AS url
  FROM (SELECT * FROM main.bronze_raw_events WHERE event_date >= '2026-04-10' AND event_date < '2026-04-11')

  ) _smelt_typed
COMMIT
```

Three things to notice, because every later page builds on them:

1. **It's a `DELETE` + `INSERT` pair over literal bounds,** wrapped in a
   transaction: throw away the partitions in the window, recompute them,
   commit both together. No merge machinery, no hidden state — a
   partition is either the full output of its query or absent.
2. **The read matches the write.** The inner `SELECT` reads
   `bronze_raw_events` filtered to exactly the same `[2026-04-10,
   2026-04-11)` window it is rebuilding. For this simple model, one day
   in means one day read — smelt derived that (trivially, here) from the
   query. The next page makes the derivation earn its keep.
3. **What you see is what runs.** The statements above are not a
   simplified rendering; they are the statements. If you've debugged a
   dbt incremental model by mentally expanding a materialization macro,
   or a Spark job by re-reading the writer options, this is the part
   smelt refuses to hide.

One honest caveat while it's cheap to say: `DELETE`+`INSERT` per partition
is a *recompute* strategy. It buys idempotence — re-running any window is
always safe — at the cost of rewriting a whole partition to change one
row. Everything smelt derives in the following pages is about keeping the
set of partitions it must rewrite as small as it can prove correct.
