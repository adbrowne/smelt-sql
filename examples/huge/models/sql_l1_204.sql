---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    created_at,
    cost,
    product_id,
    duration_seconds
FROM smelt.clicks
WHERE created_at >= '2024-01-01'
