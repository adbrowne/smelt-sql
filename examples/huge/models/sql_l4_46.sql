---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    duration_seconds,
    SUM(revenue) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.ref('py_l3_466')
GROUP BY duration_seconds
HAVING COUNT(*) > 10
