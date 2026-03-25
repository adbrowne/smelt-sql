---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_date,
    AVG(amount) AS val_1,
    SUM(amount) AS val_2
FROM smelt.ref('py_l3_446')
GROUP BY event_date
HAVING COUNT(*) > 10
