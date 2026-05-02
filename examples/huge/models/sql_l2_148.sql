---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    duration_seconds,
    campaign_id,
    LAG(amount, 1) OVER (PARTITION BY duration_seconds ORDER BY created_at) AS win_val
FROM smelt.models.sql_l1_194

