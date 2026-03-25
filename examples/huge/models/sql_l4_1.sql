---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    session_id,
    is_verified,
    discount
FROM smelt.ref('py_l3_456')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l3_449') WHERE created_at >= '2024-01-01'
)
