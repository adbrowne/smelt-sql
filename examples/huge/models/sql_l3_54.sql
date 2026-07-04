---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.revenue,
    b.user_id,
    c.browser,
    c.quantity
FROM smelt.sql_l2_247 a
INNER JOIN smelt.sql_l2_47 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_168 c ON a.user_id = c.user_id
