---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    duration_seconds,
    discount,
    segment,
    cost
FROM smelt.models.sql_l2_49
WHERE platform = 'web'

