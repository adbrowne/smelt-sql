---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    status,
    MAX(created_at) AS val_1,
    AVG(price) AS val_2
FROM smelt.ref('py_l3_480')
GROUP BY status
HAVING COUNT(*) > 10
