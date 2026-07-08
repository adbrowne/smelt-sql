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
    region,
    price
FROM smelt.sql_l1_98
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_80 WHERE platform = 'web'
)
