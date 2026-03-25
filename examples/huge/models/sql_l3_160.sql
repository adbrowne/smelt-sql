---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    platform,
    AVG(duration_seconds) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.ref('py_l2_438')
GROUP BY platform
HAVING COUNT(*) > 10
