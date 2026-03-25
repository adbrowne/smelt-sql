---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    score,
    plan_type,
    discount
FROM smelt.ref('sql_l1_173')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l1_54') WHERE is_active = true
)
