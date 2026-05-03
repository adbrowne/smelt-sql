---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    score,
    SUM(amount) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.sql_l1_207
GROUP BY score
HAVING COUNT(*) > 10

