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
    cohort_date,
    channel,
    price
FROM smelt.errors
WHERE user_id IN (
    SELECT user_id FROM smelt.errors WHERE event_type = 'purchase'
)
