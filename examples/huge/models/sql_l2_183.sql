---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    amount,
    COUNT(DISTINCT user_id) AS val_1,
    AVG(amount) AS val_2
FROM smelt.ref('py_l1_328')
GROUP BY amount
HAVING COUNT(*) > 10
