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
    a.order_id,
    a.product_id,
    b.os_name
FROM smelt.sql_l2_19 a
LEFT JOIN smelt.sql_l2_101 b ON a.user_id = b.user_id
