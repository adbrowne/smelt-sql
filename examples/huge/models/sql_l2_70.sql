---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    order_id,
    user_id,
    os_name
FROM smelt.ref('py_l1_255')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l1_192') WHERE created_at >= '2024-01-01'
)
