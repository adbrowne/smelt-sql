---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cost,
    AVG(duration_seconds) AS val_1,
    AVG(amount) AS val_2
FROM smelt.ref('py_l3_454')
GROUP BY cost
HAVING COUNT(*) > 10
