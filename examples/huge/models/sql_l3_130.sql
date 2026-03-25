---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    created_at,
    platform,
    plan_type
FROM smelt.ref('sql_l2_107')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l2_199') WHERE score >= 50
)
