---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    device_type,
    cost,
    RANK() OVER (PARTITION BY device_type ORDER BY created_at) AS win_val
FROM smelt.sql_l3_160

