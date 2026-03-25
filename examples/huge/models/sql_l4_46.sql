---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    duration_seconds,
    SUM(revenue) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.ref('sql_l3_89')
GROUP BY duration_seconds
HAVING COUNT(*) > 10
