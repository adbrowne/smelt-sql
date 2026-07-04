---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.segment,
    b.user_id,
    c.session_id,
    c.referrer
FROM smelt.sql_l1_15 a
INNER JOIN smelt.sql_l1_117 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l1_92 c ON a.user_id = c.user_id
