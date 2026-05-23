---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    channel,
    AVG(amount) AS val_1,
    COUNT(*) AS val_2
FROM smelt.sql_l2_131
GROUP BY channel
HAVING COUNT(*) > 10
