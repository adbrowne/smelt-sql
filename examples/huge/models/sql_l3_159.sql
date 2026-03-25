---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    price,
    MAX(created_at) AS val_1,
    COUNT(*) AS val_2
FROM smelt.ref('sql_l2_81')
GROUP BY price
HAVING COUNT(*) > 10
