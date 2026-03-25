---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cost,
    COUNT(*) AS val_1,
    MIN(created_at) AS val_2
FROM smelt.ref('py_l1_378')
GROUP BY cost
HAVING COUNT(*) > 10
