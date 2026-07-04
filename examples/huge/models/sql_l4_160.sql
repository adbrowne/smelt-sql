---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.profit,
    a.product_id,
    b.created_at
FROM smelt.sql_l3_175 a
INNER JOIN smelt.sql_l3_162 b ON a.user_id = b.user_id
