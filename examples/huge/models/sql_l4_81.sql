---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_verified,
    event_type,
    category
FROM smelt.ref('py_l3_368')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l3_129') WHERE is_active = true
)
