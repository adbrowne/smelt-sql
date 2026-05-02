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
    referrer,
    is_active,
    order_id
FROM smelt.models.sql_l1_75
WHERE score >= 50

