---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    plan_type,
    updated_at,
    ROW_NUMBER() OVER (PARTITION BY plan_type ORDER BY created_at) AS win_val
FROM smelt.ref('py_l2_494')
