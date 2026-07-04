---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    referrer,
    cohort_date,
    duration_seconds,
    device_type
FROM smelt.sql_l3_170
WHERE country = 'US'
