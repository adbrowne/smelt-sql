---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    created_at,
    os_name,
    ip_address
FROM smelt.ref('py_l1_452')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('py_l1_350') WHERE created_at >= '2024-01-01'
)
