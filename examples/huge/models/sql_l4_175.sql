---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    created_at,
    event_time,
    session_id
FROM smelt.ref('py_l3_385')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l3_57') WHERE quantity > 0
)
