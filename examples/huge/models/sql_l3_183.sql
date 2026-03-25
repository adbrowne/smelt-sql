---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cost,
    SUM(quantity) AS val_1,
    AVG(amount) AS val_2
FROM smelt.ref('sql_l2_44')
GROUP BY cost
HAVING COUNT(*) > 10
