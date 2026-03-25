---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    campaign_id,
    updated_at,
    cost,
    plan_type
FROM smelt.ref('py_l2_406')
WHERE platform = 'web'
