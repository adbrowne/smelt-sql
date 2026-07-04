---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    event_type,
    cohort_date,
    category,
    referrer
FROM smelt.orders
WHERE is_active = true
