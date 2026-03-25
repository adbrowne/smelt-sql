---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    plan_type,
    country,
    profit,
    category
FROM smelt.ref('sql_l2_197')
WHERE quantity > 0
