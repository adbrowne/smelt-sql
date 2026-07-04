---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.amount,
    a.discount,
    b.status
FROM smelt.sql_l3_53 a
LEFT JOIN smelt.sql_l3_53 b ON a.user_id = b.user_id
