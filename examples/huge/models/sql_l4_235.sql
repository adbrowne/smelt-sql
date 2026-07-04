---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.created_at,
    a.page_path,
    b.price
FROM smelt.sql_l3_192 a
LEFT JOIN smelt.sql_l3_239 b ON a.user_id = b.user_id
