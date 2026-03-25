---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    duration_seconds,
    plan_type,
    category
FROM smelt.ref('py_l2_260')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l2_294') WHERE status = 'active'
)
