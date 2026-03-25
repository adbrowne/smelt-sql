---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    created_at,
    quantity,
    product_id
FROM smelt.ref('sql_l1_113')
WHERE user_id IN (
    SELECT user_id FROM smelt.ref('sql_l1_154') WHERE score >= 50
)
