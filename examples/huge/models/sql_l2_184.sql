---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    score,
    plan_type,
    discount
FROM smelt.ref('sql_l1_140')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l1_344') WHERE is_active = true
)
