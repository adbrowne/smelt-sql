---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    price,
    device_type,
    RANK() OVER (PARTITION BY price ORDER BY created_at) AS win_val
FROM smelt.sql_l2_170
