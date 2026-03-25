---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    COUNT(DISTINCT user_id) AS val_1,
    AVG(price) AS val_2
FROM smelt.ref('py_l3_488')
GROUP BY status
HAVING COUNT(*) > 10
