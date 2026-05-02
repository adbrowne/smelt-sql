---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    channel,
    SUM(amount) AS val_1,
    AVG(duration_seconds) AS val_2
FROM smelt.models.sql_l1_71
GROUP BY channel
HAVING COUNT(*) > 10

