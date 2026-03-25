---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    platform,
    MAX(created_at) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.ref('py_l1_347')
GROUP BY platform
HAVING COUNT(*) > 10
