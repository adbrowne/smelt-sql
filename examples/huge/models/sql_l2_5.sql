---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.segment,
    a.cohort_date,
    b.category
FROM smelt.sql_l1_182 a
INNER JOIN smelt.sql_l1_182 b ON a.user_id = b.user_id
