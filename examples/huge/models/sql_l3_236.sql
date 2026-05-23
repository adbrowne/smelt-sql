---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    campaign_id,
    duration_seconds,
    LAG(amount, 1) OVER (PARTITION BY campaign_id ORDER BY created_at) AS win_val
FROM smelt.sql_l2_139
