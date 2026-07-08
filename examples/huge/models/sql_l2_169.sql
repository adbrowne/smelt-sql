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
    updated_at,
    created_at
FROM smelt.sql_l1_156
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_108 WHERE country = 'US'
)
