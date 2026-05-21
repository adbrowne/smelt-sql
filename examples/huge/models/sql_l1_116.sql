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
    event_date,
    COUNT(DISTINCT user_id) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.signups
GROUP BY event_date
HAVING COUNT(*) > 10

