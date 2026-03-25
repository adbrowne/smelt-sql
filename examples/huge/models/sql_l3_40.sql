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
    plan_type,
    profit
FROM smelt.ref('sql_l2_65')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_63') WHERE status = 'active'
)
