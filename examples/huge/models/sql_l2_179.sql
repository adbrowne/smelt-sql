---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    discount,
    tier,
    created_at,
    rating
FROM smelt.ref('py_l1_467')
WHERE score >= 50
