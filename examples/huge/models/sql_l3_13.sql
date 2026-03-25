---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_type,
    MAX(created_at) AS val_1,
    MIN(created_at) AS val_2
FROM smelt.ref('py_l2_334')
GROUP BY event_type
HAVING COUNT(*) > 10
