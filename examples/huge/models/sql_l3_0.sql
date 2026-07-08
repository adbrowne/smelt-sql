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
    country,
    price,
    is_active
FROM smelt.sql_l2_109
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_225 WHERE platform = 'web'
)
