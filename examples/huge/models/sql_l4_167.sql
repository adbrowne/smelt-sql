---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    quantity,
    SUM(quantity) AS val_1,
    AVG(price) AS val_2
FROM smelt.ref('sql_l3_6')
GROUP BY quantity
HAVING COUNT(*) > 10
