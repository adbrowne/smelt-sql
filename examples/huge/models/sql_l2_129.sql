---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    is_active,
    AVG(duration_seconds) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.ref('py_l1_446')
GROUP BY is_active
HAVING COUNT(*) > 10
