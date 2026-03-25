---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    transaction_id,
    SUM(quantity) AS val_1,
    SUM(amount) AS val_2
FROM smelt.ref('py_l2_313')
GROUP BY transaction_id
HAVING COUNT(*) > 10
