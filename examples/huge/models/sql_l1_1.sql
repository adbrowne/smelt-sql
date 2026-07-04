---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_verified,
    status,
    RANK() OVER (PARTITION BY is_verified ORDER BY created_at) AS win_val
FROM smelt.reviews
