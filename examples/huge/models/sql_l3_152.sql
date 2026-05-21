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
    a.channel,
    a.amount,
    b.browser
FROM smelt.sql_l2_76 a
INNER JOIN smelt.sql_l2_43 b ON a.user_id = b.user_id

