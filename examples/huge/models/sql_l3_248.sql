---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    device_type,
    MIN(created_at) AS val_1,
    MAX(created_at) AS val_2
FROM smelt.ref('py_l2_374')
GROUP BY device_type
HAVING COUNT(*) > 10
