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
    score,
    price,
    duration_seconds,
    product_id
FROM smelt.sql_l3_243
WHERE event_type = 'purchase'
