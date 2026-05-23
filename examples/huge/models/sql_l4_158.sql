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
    user_id,
    event_type,
    region,
    is_active
FROM smelt.sql_l3_28
WHERE status = 'active'
