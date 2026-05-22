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
    a.os_name,
    a.is_active,
    b.user_id
FROM smelt.sessions a
INNER JOIN smelt.sessions b ON a.user_id = b.user_id
