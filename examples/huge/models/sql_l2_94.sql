---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    amount,
    COUNT(*) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.ref('sql_l1_180')
GROUP BY amount
HAVING COUNT(*) > 10
