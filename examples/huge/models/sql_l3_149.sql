---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.user_id,
    a.quantity,
    b.product_id
FROM smelt.sql_l2_92 a
INNER JOIN smelt.sql_l2_70 b ON a.user_id = b.user_id
