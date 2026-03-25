---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.device_type,
    a.order_id,
    b.quantity
FROM smelt.ref('errors') a
INNER JOIN smelt.ref('errors') b ON a.user_id = b.user_id
