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
    a.segment,
    a.cohort_date,
    b.amount
FROM smelt.sql_l2_9 a
INNER JOIN smelt.sql_l2_8 b ON a.user_id = b.user_id

