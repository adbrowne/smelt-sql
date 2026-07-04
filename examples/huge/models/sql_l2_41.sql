---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.discount,
    b.session_id,
    c.ip_address,
    c.quantity
FROM smelt.sql_l1_91 a
INNER JOIN smelt.sql_l1_91 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l1_91 c ON a.user_id = c.user_id
