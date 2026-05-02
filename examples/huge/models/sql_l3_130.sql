---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    created_at,
    platform,
    plan_type
FROM smelt.models.sql_l2_153
WHERE user_id IN (
    SELECT user_id FROM smelt.models.sql_l2_31 WHERE score >= 50
)

