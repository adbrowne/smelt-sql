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
    is_verified,
    referrer,
    device_type,
    category
FROM smelt.sql_l3_130
WHERE is_active = true

