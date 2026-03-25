---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    quantity,
    score,
    event_type
FROM smelt.ref('sql_l2_146')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l2_254') WHERE status = 'active'
)
