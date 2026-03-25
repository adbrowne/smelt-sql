---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    duration_seconds,
    discount,
    segment,
    cost
FROM smelt.ref('py_l2_291')
WHERE platform = 'web'
