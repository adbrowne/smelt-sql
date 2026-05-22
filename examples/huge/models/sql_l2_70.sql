---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    order_id,
    user_id,
    os_name
FROM smelt.sql_l1_62
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l1_62 WHERE created_at >= '2024-01-01'
)
