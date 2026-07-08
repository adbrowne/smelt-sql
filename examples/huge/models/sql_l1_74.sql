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
    event_date,
    order_id,
    country,
    is_active
FROM smelt.reviews
WHERE is_active = true
