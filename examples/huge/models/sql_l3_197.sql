---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    session_id,
    plan_type,
    event_type
FROM smelt.sql_l2_13
WHERE user_id IN (
    SELECT user_id FROM smelt.sql_l2_99 WHERE category IS NOT NULL
)

