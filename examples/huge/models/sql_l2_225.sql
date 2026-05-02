---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    tier,
    AVG(duration_seconds) AS val_1,
    SUM(amount) AS val_2
FROM smelt.models.sql_l1_37
GROUP BY tier
HAVING COUNT(*) > 10

