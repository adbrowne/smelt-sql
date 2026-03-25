---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    platform,
    SUM(revenue) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.ref('py_l2_451')
GROUP BY platform
HAVING COUNT(*) > 10
