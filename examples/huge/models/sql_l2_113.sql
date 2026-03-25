---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    user_id,
    cost,
    discount
FROM smelt.ref('sql_l1_177')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l1_134') WHERE status = 'active'
)
