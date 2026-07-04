---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    a.cohort_date,
    a.updated_at,
    b.is_active
FROM smelt.sql_l1_25 a
LEFT JOIN smelt.sql_l1_99 b ON a.user_id = b.user_id
