---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_active,
    referrer,
    country,
    price
FROM smelt.ref('py_l2_257')
WHERE platform = 'web'
