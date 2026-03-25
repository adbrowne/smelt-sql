---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    plan_type,
    created_at,
    quantity,
    score
FROM smelt.ref('py_l3_477')
WHERE status = 'active'
