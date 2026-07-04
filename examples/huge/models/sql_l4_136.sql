---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    os_name,
    ip_address,
    is_active
FROM smelt.sql_l3_119
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l3_119 WHERE quantity > 0
)
