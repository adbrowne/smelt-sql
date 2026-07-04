---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    is_verified,
    event_type,
    cohort_date,
    rating
FROM smelt.sql_l2_151
WHERE status = 'active'
