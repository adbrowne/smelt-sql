---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    country,
    referrer,
    created_at,
    event_time
FROM smelt.ref('py_l3_394')
WHERE quantity > 0
