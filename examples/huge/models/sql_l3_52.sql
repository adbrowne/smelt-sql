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
    a.product_id,
    a.amount,
    b.quantity
FROM smelt.sql_l2_219 a
LEFT JOIN smelt.sql_l2_7 b ON a.user_id = b.user_id
