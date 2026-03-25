---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    platform,
    plan_type,
    transaction_id,
    price
FROM smelt.ref('py_l2_397')
WHERE platform = 'web'
