---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    os_name,
    AVG(duration_seconds) AS val_1,
    COUNT(*) AS val_2
FROM smelt.ref('py_l1_385')
GROUP BY os_name
HAVING COUNT(*) > 10
