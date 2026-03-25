---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    cost,
    AVG(price) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.ref('sql_l1_241')
GROUP BY cost
HAVING COUNT(*) > 10
