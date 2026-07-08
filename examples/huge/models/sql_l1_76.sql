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
    score,
    revenue,
    LAG(amount, 1) OVER (PARTITION BY score ORDER BY created_at) AS win_val
FROM smelt.page_views
