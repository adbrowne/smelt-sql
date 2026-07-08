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
    revenue,
    status,
    rating,
    tier
FROM smelt.sql_l3_176
WHERE event_type = 'purchase'
