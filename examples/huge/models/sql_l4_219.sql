---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.event_time,
    b.channel,
    c.is_active,
    c.cohort_date
FROM smelt.sql_l3_188 a
INNER JOIN smelt.sql_l3_114 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l3_188 c ON a.user_id = c.user_id
