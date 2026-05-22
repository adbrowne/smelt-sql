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
    amount,
    COUNT(DISTINCT user_id) AS val_1,
    AVG(amount) AS val_2
FROM smelt.sql_l1_84
GROUP BY amount
HAVING COUNT(*) > 10
