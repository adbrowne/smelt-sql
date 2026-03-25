---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    session_id,
    country,
    updated_at
FROM smelt.ref('py_l2_415')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_120') WHERE event_type = 'purchase'
)
