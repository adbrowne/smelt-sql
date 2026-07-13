<!-- GENERATED FILE — edit examples/web_analytics/tutorial_pages/late-data.md and run python3 examples/web_analytics/generate_tutorial.py -->

# Duplicates and late data

<!-- PLACEHOLDER: intro prose. -->

## Deduplicating redeliveries — and a refusal

<!-- smelt-include: tutorial_stages/02_dedup_refused/models/silver/events_parsed.sql -->
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
QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_time) = 1
```

```bash
smelt run --select silver.events_parsed \
  --event-time-start 2026-04-10 --event-time-end 2026-04-11 --dry-run
```

<!-- smelt-generate: @cwd=tutorial_stages/02_dedup_refused @render=text @expect-exit=1 run --select silver.events_parsed --event-time-start 2026-04-10 --event-time-end 2026-04-11 --dry-run -->
```text
Error: Incremental safety check refused the following model(s). Fix the SQL or use --allow-downgrade to fall back to full-table refresh:
  • Model 'events_parsed': window function with OVER clause is not compatible with incremental materialization — window OVER (PARTITION BY event_id) does not include the partition_column 'event_date'. Use OVER (PARTITION BY event_date ...) to make it partition-aligned, or set safety_overrides.allow_window_functions: true
```

<!-- PLACEHOLDER: why refused, why the override is justified. -->

## Accepting late arrivals

<!-- smelt-include: tutorial_stages/03_late_data/models/silver/events_parsed.sql -->
```sql
---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_date
  partition_column: event_date
  granularity: day
# The redelivery dedup window partitions by event_id, not event_date — the
# analyzer cannot statically prove it is partition-aligned. It is safe in
# practice: a redelivered duplicate always lands in the *same* event_date
# partition as its original, so the window never needs to see across a
# partition boundary to resolve one event_id's duplicates.
batched:
  safety_overrides:
    allow_window_functions: true
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
WHERE event_date
    BETWEEN CAST(arrival_time AS DATE) - INTERVAL '3 days'
        AND CAST(arrival_time AS DATE)
QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_time) = 1
```

## The derived lookback

<!-- smelt-generate: @cwd=tutorial_stages/03_late_data explain silver.events_parsed --show-sql --json --period 2026-04-10..2026-04-11 -->
```sql
-- trigger: Backfill
BEGIN
  DELETE FROM main.silver_events_parsed WHERE event_date >= '2026-04-10' AND event_date < '2026-04-11'
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
  FROM (SELECT * FROM main.bronze_raw_events WHERE event_date >= '2026-04-07' AND event_date < '2026-04-11')
  WHERE event_date
      BETWEEN CAST(arrival_time AS DATE) - INTERVAL '3 days'
          AND CAST(arrival_time AS DATE)
  QUALIFY ROW_NUMBER() OVER (PARTITION BY event_id ORDER BY arrival_time) = 1

  ) AS _smelt_output_clamp WHERE event_date >= '2026-04-10' AND event_date < '2026-04-11'
COMMIT
```

<!-- PLACEHOLDER: the [D-3, D+1) read window, dbt/Spark contrast. -->
