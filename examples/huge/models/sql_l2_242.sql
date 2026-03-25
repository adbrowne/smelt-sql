---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    rating,
    ip_address,
    LAG(amount, 1) OVER (PARTITION BY rating ORDER BY created_at) AS win_val
FROM smelt.ref('py_l1_404')
