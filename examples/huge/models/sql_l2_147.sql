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
    a.referrer,
    b.created_at,
    c.transaction_id,
    c.category
FROM smelt.sql_l1_18 a
INNER JOIN smelt.sql_l1_232 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l1_18 c ON a.user_id = c.user_id
