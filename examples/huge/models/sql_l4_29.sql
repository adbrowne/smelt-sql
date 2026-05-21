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
    a.channel,
    b.status
FROM smelt.sql_l3_195 a
INNER JOIN smelt.sql_l3_67 b ON a.user_id = b.user_id

