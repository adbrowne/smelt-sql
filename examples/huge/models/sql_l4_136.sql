---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    os_name,
    ip_address,
    is_active
FROM smelt.ref('sql_l3_59')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l3_294') WHERE quantity > 0
)
