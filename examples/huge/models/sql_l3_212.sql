---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cost,
    user_id,
    ip_address,
    session_id
FROM smelt.ref('py_l2_420')
WHERE status = 'active'
