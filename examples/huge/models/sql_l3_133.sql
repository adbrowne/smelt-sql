---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    country,
    duration_seconds,
    score,
    is_active
FROM smelt.ref('sql_l2_196')
WHERE platform = 'web'
