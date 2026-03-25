---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    discount,
    AVG(duration_seconds) AS val_1,
    MIN(created_at) AS val_2
FROM smelt.ref('py_l1_287')
GROUP BY discount
HAVING COUNT(*) > 10
