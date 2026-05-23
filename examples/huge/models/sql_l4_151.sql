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
    category,
    is_active,
    channel,
    created_at
FROM smelt.sql_l3_153
WHERE is_active = true
