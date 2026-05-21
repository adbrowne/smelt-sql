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
    a.user_id,
    b.revenue,
    c.quantity,
    c.rating
FROM smelt.sql_l3_11 a
INNER JOIN smelt.sql_l3_11 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l3_11 c ON a.user_id = c.user_id

