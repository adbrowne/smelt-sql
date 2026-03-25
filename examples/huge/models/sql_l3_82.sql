---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    revenue,
    event_time,
    LAG(amount, 1) OVER (PARTITION BY revenue ORDER BY created_at) AS win_val
FROM smelt.ref('sql_l2_198')
