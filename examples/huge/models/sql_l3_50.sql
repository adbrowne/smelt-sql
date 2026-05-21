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
    a.event_date,
    b.user_id
FROM smelt.sql_l2_53 a
LEFT JOIN smelt.sql_l2_89 b ON a.user_id = b.user_id

