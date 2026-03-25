---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    duration_seconds,
    referrer,
    is_active,
    order_id
FROM smelt.ref('py_l1_375')
WHERE score >= 50
