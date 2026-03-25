---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    score,
    SUM(amount) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.ref('sql_l1_120')
GROUP BY score
HAVING COUNT(*) > 10
