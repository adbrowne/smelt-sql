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
    event_time,
    os_name,
    transaction_id
FROM smelt.sql_l1_173
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_245 WHERE country = 'US'
)
