---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    revenue,
    status,
    LAG(amount, 1) OVER (PARTITION BY revenue ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l3_238')
