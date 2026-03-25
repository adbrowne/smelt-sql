---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    event_type,
    AVG(duration_seconds) AS val_1,
    MAX(created_at) AS val_2
FROM smelt.ref('py_l3_350')
GROUP BY event_type
HAVING COUNT(*) > 10
