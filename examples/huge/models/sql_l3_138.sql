---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    browser,
    revenue,
    duration_seconds,
    device_type
FROM smelt.models.sql_l2_46
WHERE status = 'active'

