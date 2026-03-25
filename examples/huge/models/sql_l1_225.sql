---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.device_type,
    a.order_id,
    b.quantity
FROM smelt.ref('errors') a
INNER JOIN smelt.ref('errors') b ON a.user_id = b.user_id
