---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    country,
    duration_seconds,
    score,
    is_active
FROM smelt.ref('py_l2_344')
WHERE platform = 'web'
