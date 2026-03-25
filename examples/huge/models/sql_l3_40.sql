---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    duration_seconds,
    plan_type,
    profit
FROM smelt.ref('sql_l2_48')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_52') WHERE status = 'active'
)
