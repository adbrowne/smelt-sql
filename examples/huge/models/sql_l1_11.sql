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
    user_id,
    COUNT(DISTINCT user_id) AS val_1,
    SUM(amount) AS val_2
FROM smelt.notifications
GROUP BY user_id
HAVING COUNT(*) > 10

