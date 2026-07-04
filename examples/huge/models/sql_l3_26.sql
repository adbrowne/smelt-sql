---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.region,
    a.platform,
    b.segment
FROM smelt.sql_l2_140 a
INNER JOIN smelt.sql_l2_131 b ON a.user_id = b.user_id
