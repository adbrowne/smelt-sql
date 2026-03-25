---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    product_id,
    COUNT(DISTINCT user_id) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.ref('py_l2_349')
GROUP BY product_id
HAVING COUNT(*) > 10
