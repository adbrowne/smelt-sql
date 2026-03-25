---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    ip_address,
    device_type,
    order_id
FROM smelt.ref('users')
WHERE status = 'active'
