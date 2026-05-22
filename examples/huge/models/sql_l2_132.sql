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
    a.segment,
    b.updated_at,
    c.referrer,
    c.price
FROM smelt.sql_l1_43 a
INNER JOIN smelt.sql_l1_43 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l1_43 c ON a.user_id = c.user_id
