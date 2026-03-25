---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    cost,
    product_id
FROM smelt.ref('sql_l1_134')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l1_94') WHERE event_type = 'purchase'
)
