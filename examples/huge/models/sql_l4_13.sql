---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    product_id,
    COUNT(*) AS val_1,
    MAX(created_at) AS val_2
FROM smelt.ref('py_l3_265')
GROUP BY product_id
HAVING COUNT(*) > 10
