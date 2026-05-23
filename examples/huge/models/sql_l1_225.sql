---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.device_type,
    a.order_id,
    b.quantity
FROM smelt.errors a
INNER JOIN smelt.errors b ON a.user_id = b.user_id
