---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.country,
    b.created_at,
    c.cohort_date,
    c.discount
FROM smelt.sql_l2_227 a
INNER JOIN smelt.sql_l2_163 b ON a.user_id = b.user_id
LEFT JOIN smelt.sql_l2_160 c ON a.user_id = c.user_id
