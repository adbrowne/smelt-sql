---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    region,
    COUNT(*) AS val_1,
    AVG(amount) AS val_2
FROM smelt.ref('py_l2_391')
GROUP BY region
HAVING COUNT(*) > 10
