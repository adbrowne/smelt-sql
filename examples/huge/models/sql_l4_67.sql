---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    revenue,
    status,
    LAG(amount, 1) OVER (PARTITION BY revenue ORDER BY created_at) AS win_val
FROM smelt.ref('py_l3_392')
