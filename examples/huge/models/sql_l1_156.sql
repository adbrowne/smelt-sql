---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    channel,
    country,
    LAG(amount, 1) OVER (PARTITION BY channel ORDER BY created_at) AS win_val
FROM smelt.signups
