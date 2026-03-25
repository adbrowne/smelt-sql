---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    score,
    region,
    price
FROM smelt.ref('sql_l1_94')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l1_94') WHERE platform = 'web'
)
