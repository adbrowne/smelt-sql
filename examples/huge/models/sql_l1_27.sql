---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    profit,
    os_name,
    device_type
FROM smelt.ref('users')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('users') WHERE created_at >= '2024-01-01'
)
