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
    cost,
    user_id,
    ip_address,
    session_id
FROM smelt.sql_l2_99
WHERE status = 'active'
