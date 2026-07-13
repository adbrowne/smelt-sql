<!-- GENERATED FILE — edit examples/web_analytics/tutorial_pages/first-model.md and run python3 examples/web_analytics/generate_tutorial.py -->

# A first incremental model

<!-- PLACEHOLDER: intro prose. -->

## The model

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

<!-- PLACEHOLDER: frontmatter walkthrough, run commands. -->

```bash
smelt build --event-time-start 2026-04-10 --event-time-end 2026-04-11
```

## What actually runs

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

<!-- PLACEHOLDER: DELETE+INSERT walkthrough. -->
