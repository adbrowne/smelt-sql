---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    order_id,
    SUM(quantity) AS val_1,
    AVG(amount) AS val_2
FROM smelt.ref('py_l2_474')
GROUP BY order_id
HAVING COUNT(*) > 10
