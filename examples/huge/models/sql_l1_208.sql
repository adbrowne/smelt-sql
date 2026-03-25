---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    a.quantity,
    a.product_id,
    b.cost
FROM smelt.ref('categories') a
INNER JOIN smelt.ref('categories') b ON a.user_id = b.user_id
