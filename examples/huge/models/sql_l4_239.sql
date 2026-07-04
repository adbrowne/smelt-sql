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
    b.segment,
    c.tier,
    c.channel
FROM smelt.sql_l3_56 a
INNER JOIN smelt.sql_l3_5 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l3_140 c ON a.user_id = c.user_id
