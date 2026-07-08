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
    status,
    referrer,
    duration_seconds
FROM smelt.reviews
WHERE user_id IN (
    SELECT user_id FROM smelt.reviews WHERE event_type = 'purchase'
)
