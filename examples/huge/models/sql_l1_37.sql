---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    rating,
    duration_seconds,
    RANK() OVER (PARTITION BY rating ORDER BY created_at) AS win_val
FROM smelt.page_views
