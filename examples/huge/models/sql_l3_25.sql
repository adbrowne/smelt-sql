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
    a.transaction_id,
    b.profit,
    c.status,
    c.discount
FROM smelt.sql_l2_11 a
INNER JOIN smelt.sql_l2_68 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_44 c ON a.user_id = c.user_id

