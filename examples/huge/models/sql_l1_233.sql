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
    event_type,
    platform,
    transaction_id,
    quantity
FROM smelt.products
WHERE score >= 50
