---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    plan_type,
    created_at,
    quantity,
    score
FROM smelt.sql_l3_40
WHERE status = 'active'

