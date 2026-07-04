---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    status,
    category,
    rating,
    event_type
FROM smelt.subscriptions
WHERE category IS NOT NULL
