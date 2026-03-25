---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    device_type,
    score,
    LAG(amount, 1) OVER (PARTITION BY device_type ORDER BY created_at) AS win_val
FROM smelt.ref('py_l1_264')
