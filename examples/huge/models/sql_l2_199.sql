---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    revenue,
    MIN(created_at) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.ref('py_l1_330')
GROUP BY revenue
HAVING COUNT(*) > 10
