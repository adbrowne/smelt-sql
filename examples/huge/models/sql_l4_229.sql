---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    profit,
    status,
    LAG(amount, 1) OVER (PARTITION BY profit ORDER BY created_at) AS win_val
FROM smelt.models.sql_l3_117

