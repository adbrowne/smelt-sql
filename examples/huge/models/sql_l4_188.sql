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
    a.cost,
    b.price,
    c.cohort_date,
    c.tier
FROM smelt.sql_l3_143 a
INNER JOIN smelt.sql_l3_143 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l3_143 c ON a.user_id = c.user_id
