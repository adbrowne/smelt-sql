---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    country,
    MAX(created_at) AS val_1,
    SUM(amount) AS val_2
FROM smelt.ref('py_l2_269')
GROUP BY country
HAVING COUNT(*) > 10
