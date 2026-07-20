<!-- GENERATED FILE — edit examples/web_analytics/tutorial_pages/changing-things.md and run python3 examples/web_analytics/generate_tutorial.py -->

# Backfills, new columns, and late updates

A pipeline that only ever moves forward one day at a time is the easy
case. Real ones change in three distinct ways, and it's worth keeping
them distinct, because the correct response to each is different:

1. **Backfill** — recompute history that already has a definition: a new
   model needs the past year, or a bug fix invalidates March.
2. **A new column** — the *shape* changes: the query grows a column that
   history never had.
3. **Late updates** — the definition and shape are fine, but upstream
   data changed: late arrivals landed, a source was corrected.

There is a fourth kind — changing the *logic* of an existing column —
that smelt does not yet detect: the schema is unchanged, so nothing
flags that history is stale, and you re-run or `backbuild` the affected
range yourself. Automatic definition-change handling is tracked in the
project's maintenance-plan spec; until it lands, treat logic edits as
backfills you initiate.

This page runs each one against the pipeline as built so far, on the
stage projects ([stage 3](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics/tutorial_stages/03_late_data),
[stage 4](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics/tutorial_stages/04_add_column),
[stage 5](https://github.com/adbrowne/smelt-sql/tree/main/examples/web_analytics/tutorial_stages/05_enrichment)).

## Backfilling a range

`smelt backbuild` rebuilds a model (and anything upstream it needs) over
a date range. `--dry-run` prints the full plan without executing:

```bash
smelt backbuild silver.events_parsed --start 2026-04-01 --end 2026-04-19 --dry-run
```

<!-- smelt-generate: @cwd=tutorial_stages/03_late_data @render=skeleton backbuild silver.events_parsed --start 2026-04-01 --end 2026-04-19 --dry-run -->
```sql
-- Would run: bronze.raw_events (materialization shown below)
-- … model SELECT body (see the full SQL below) …

) _smelt_typed

-- Would run: silver.events_parsed (materialization shown below)
-- … model SELECT body (see the full SQL below) …

) _smelt_typed

-- chunk 1/2: [2026-04-01, 2026-04-10)
BEGIN
  DELETE FROM main.silver_events_parsed WHERE event_date >= '2026-04-01' AND event_date < '2026-04-10'
  INSERT INTO main.silver_events_parsed SELECT * FROM (
-- … model SELECT body (see the full SQL below) …

) AS _smelt_output_clamp WHERE event_date >= '2026-04-01' AND event_date < '2026-04-10'
COMMIT
-- chunk 2/2: [2026-04-10, 2026-04-19)
BEGIN
  DELETE FROM main.silver_events_parsed WHERE event_date >= '2026-04-10' AND event_date < '2026-04-19'
  INSERT INTO main.silver_events_parsed SELECT * FROM (
-- … model SELECT body (see the full SQL below) …

) AS _smelt_output_clamp WHERE event_date >= '2026-04-10' AND event_date < '2026-04-19'
COMMIT
```

??? example "Full dry-run transcript"

    <!-- smelt-generate: @cwd=tutorial_stages/03_late_data backbuild silver.events_parsed --start 2026-04-01 --end 2026-04-19 --dry-run -->
    ```sql
    -- Would run: bronze.raw_events (materialization shown below)
    SELECT CAST(event_id AS BIGINT) AS event_id, CAST(device_id AS INTEGER) AS device_id, CAST(user_id AS INTEGER) AS user_id, CAST(seconds_in_day AS INTEGER) AS seconds_in_day, CAST(event_time AS TIMESTAMP) AS event_time, CAST(arrival_time AS TIMESTAMP) AS arrival_time, CAST(utm_campaign AS VARCHAR) AS utm_campaign, CAST(payload AS VARCHAR) AS payload, CAST(event_date AS DATE) AS event_date FROM (
    SELECT
        event_id,
        device_id,
        user_id,
        seconds_in_day,
        CAST(event_time AS TIMESTAMP) AS event_time,
        CAST(arrival_time AS TIMESTAMP) AS arrival_time,
        utm_campaign,
        payload,
        CAST(event_date AS DATE) AS event_date
    FROM raw.events
    
    ) _smelt_typed
    
    -- Would run: silver.events_parsed (materialization shown below)
    SELECT CAST(event_id AS BIGINT) AS event_id, CAST(device_id AS INTEGER) AS device_id, CAST(user_id AS INTEGER) AS user_id, CAST(amplitude_id AS VARCHAR) AS amplitude_id, CAST(event_ts AS TIMESTAMP) AS event_ts, CAST(event_date AS DATE) AS event_date, CAST(utm_campaign AS VARCHAR) AS utm_campaign, CAST(event_name AS VARCHAR) AS event_name, CAST(platform AS VARCHAR) AS platform, CAST(url AS VARCHAR) AS url FROM (
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
    FROM main.bronze_raw_events
    WHERE event_date
        BETWEEN CAST(arrival_time AS DATE) - INTERVAL '3 days'
            AND CAST(arrival_time AS DATE)
    QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_time) = 1
    
    ) _smelt_typed
    
    -- chunk 1/2: [2026-04-01, 2026-04-10)
    BEGIN
      DELETE FROM main.silver_events_parsed WHERE event_date >= '2026-04-01' AND event_date < '2026-04-10'
      INSERT INTO main.silver_events_parsed SELECT * FROM (
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
    FROM (SELECT * FROM main.bronze_raw_events WHERE event_date >= '2026-03-29' AND event_date < '2026-04-10')
    WHERE event_date
        BETWEEN CAST(arrival_time AS DATE) - INTERVAL '3 days'
            AND CAST(arrival_time AS DATE)
    QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_time) = 1
    
    ) AS _smelt_output_clamp WHERE event_date >= '2026-04-01' AND event_date < '2026-04-10'
    COMMIT
    -- chunk 2/2: [2026-04-10, 2026-04-19)
    BEGIN
      DELETE FROM main.silver_events_parsed WHERE event_date >= '2026-04-10' AND event_date < '2026-04-19'
      INSERT INTO main.silver_events_parsed SELECT * FROM (
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
    FROM (SELECT * FROM main.bronze_raw_events WHERE event_date >= '2026-04-07' AND event_date < '2026-04-19')
    WHERE event_date
        BETWEEN CAST(arrival_time AS DATE) - INTERVAL '3 days'
            AND CAST(arrival_time AS DATE)
    QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_time) = 1
    
    ) AS _smelt_output_clamp WHERE event_date >= '2026-04-10' AND event_date < '2026-04-19'
    COMMIT
    ```

The 18-day range didn't become one giant statement: it was split into
chunks (`-- chunk 1/2`, `-- chunk 2/2`), each its own transactional
`DELETE`+`INSERT`, so a long backfill holds bounded memory and commits
incrementally — and fails cleanly: if a later chunk dies, earlier
chunks are already durable, and re-running the backfill is safe because
each chunk's recompute is idempotent. Notice each chunk's *read* carries its own three-day
widening at the edge — `chunk 1` writes `[04-01, 04-10)` but reads bronze
from `03-29` — the late-arrival derivation from
[the late-data page](late-data.md), composing with chunking instead of
being re-derived by hand for the backfill case.

Because this model's partitions are independent, no ordering between
chunks is required for correctness. For the ordered sessionizer in
[the deep dive](ordered-sessions.md), the same command enforces
oldest-first instead — same interface, different derived discipline.

## Adding a column

Add a derived flag to `events_parsed` — one new line in the SELECT:

```sql
    CASE WHEN json_extract_string(payload, '$.event_name') = 'purchase'
         THEN TRUE
    END AS is_purchase
```

History was built without this column, so model and table now disagree.
`smelt diff` compares the model against the deployed schema and shows the
migration it implies, before anything runs:

```bash
smelt diff --select silver.events_parsed
```

<!-- smelt-generate: @cwd=tutorial_stages/04_add_column @fixture-schemas @render=text @expect-exit=1 diff --select silver.events_parsed -->
```text
Model: silver.events_parsed
  ADD COLUMN is_purchase BOOLEAN NULL
  -> Safe: ALTER TABLE (no data loss)
     ALTER TABLE main.silver_events_parsed ADD COLUMN is_purchase BOOLEAN

Summary: 1 changed, 0 new, 0 removed, 0 unchanged
Error: schema changes detected
```

The change classifies as **safe**: adding a nullable column needs no
rewrite. The next `smelt run` applies the `ALTER TABLE` automatically and
carries on incrementally — already-built partitions keep `NULL` for
`is_purchase`, and partitions built from now on populate it. (This exact
flow — baseline saved, column added, `ALTER` applied, old rows `NULL`,
new rows populated — is pinned end-to-end by
`e2e::schema_evolution_incremental` in the repo.)

Two follow-ups you'll want eventually:

- **Populating history.** `NULL` history is often fine (the flag simply
  starts existing). When it isn't, declare a `backfill:` expression for
  the column in frontmatter, or `smelt backbuild` the range — the
  definition already computes the column, so a backfill fills it. See
  [schema evolution](../../guide/schema-evolution.md).
- **The not-nullable trap.** If the new column is provably `NOT NULL`
  (say, `amount >= 100` over a non-null `amount`), an in-place `ALTER`
  is impossible — `NULL` history would violate the type — and smelt
  responds with a **full-table rebuild** on the next run. Correct, but
  on a large table you want to choose the moment. Keep intentionally
  nullable additions nullable (our `CASE` has no `ELSE` for exactly this
  reason), or declare a `default:`.

For comparison: dbt's `on_schema_change: append_new_columns` does the
`ALTER` but nothing checks nullability against history, and sync is
opt-in per model. SQLMesh's plan preview is the nearest analog to
`smelt diff` — it categorizes changes as breaking or non-breaking before
applying. Spark's `mergeSchema` appends columns on write. The difference
here is mostly that the classification extends to the physical migration
statement, shown before the run, with the unsafe variant refused rather
than attempted.

## When upstream data changes

Late arrivals were accepted on [the late-data page](late-data.md) by
*policy*; this is the mechanics of absorbing them. Suppose the feed
delivers a batch of stragglers whose `event_date` is April 10th —
bronze's April 10th partition just gained rows, today. Everything
computed *from* that day is now stale: parsed events for the 10th, any
session that day's events participate in, and every enriched row that
referenced those sessions.

You could re-run a generous window over the whole pipeline. The precise
alternative: tell smelt what landed, and it answers with a **dirty set**
— every model-and-window pair the declared delta could have made stale —
then runs exactly that, in dependency order:

```bash
smelt run --since-upstream \
  --source silver.events_parsed --landed 2026-04-10..2026-04-11 --dry-run
```

<!-- smelt-generate: @cwd=tutorial_stages/05_enrichment @render=dirty-set run --since-upstream --source silver.events_parsed --landed 2026-04-10..2026-04-11 --dry-run -->
```text
Dirty set (--since-upstream):
  silver.events_enriched <- silver.events_parsed: [2026-04-10, 2026-04-11)
  silver.events_enriched <- silver.sessions: [2026-04-08, 2026-04-14)
  silver.sessions <- silver.events_parsed: [2026-04-09, 2026-04-13)
  RUN silver.events_enriched: [2026-04-08, 2026-04-14)
  RUN silver.sessions: [2026-04-09, 2026-04-13)
```

The widening rule is the same one you've been reading all along, applied
in reverse: **a partition is dirty if its read window touches the
delta.** A `sessions` partition P reads events from two days before P
(the sessionizer's lookback) through one day after (the session span) —
so a one-day delta dirties the four session partitions `[04-09, 04-13)`.
An `events_enriched` partition reads sessions from a day either side of
itself, so those four dirty session days spread to six enriched days,
`[04-08, 04-14)`. None of it is configured in the run command; every
number is the inversion of a filter you've already seen declared.

The stage-5 project this runs against contains the enrichment model —
event rows with their session identity joined back on
([source](https://github.com/adbrowne/smelt-sql/blob/main/examples/web_analytics/tutorial_stages/05_enrichment/models/silver/events_enriched.sql)) —
but not the ordered `sessions_chained` table. That's the cost from
[the sessions page](sessions.md) arriving on schedule: propagation
refuses graphs with self-referential nodes rather than guessing at them,
so the full example (which keeps both session tables) currently answers
`--since-upstream` with a named refusal, and the workaround is the
generous-window re-run.

!!! note "Two pragmatic notes"
    `--landed` windows come from you or your loader's bookkeeping —
    smelt does not yet watermark-diff sources automatically. And for the
    simple "resume where I left off" case, `smelt run --auto` fills
    whatever intervals haven't been processed yet
    ([CLI reference](../../reference/cli.md)).

The through-line of all three changes is the same: you described *what
changed* — a range, a column, a landed window — and every bound in
smelt's response was derived from declarations the pipeline had already
made.
