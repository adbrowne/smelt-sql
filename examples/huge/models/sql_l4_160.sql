---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.profit,
    a.product_id,
    b.created_at
FROM smelt.ref('sql_l3_196') a
INNER JOIN smelt.ref('sql_l3_196') b ON a.user_id = b.user_id
