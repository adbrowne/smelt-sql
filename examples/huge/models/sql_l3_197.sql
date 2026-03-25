---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    session_id,
    plan_type,
    event_type
FROM smelt.ref('py_l2_470')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l2_492') WHERE category IS NOT NULL
)
