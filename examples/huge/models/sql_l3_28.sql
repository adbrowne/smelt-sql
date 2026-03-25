---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    updated_at,
    price,
    is_active,
    referrer
FROM smelt.ref('py_l2_473')
WHERE score >= 50
