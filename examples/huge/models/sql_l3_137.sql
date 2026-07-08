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
    platform,
    plan_type,
    transaction_id,
    price
FROM smelt.sql_l2_126
WHERE platform = 'web'
