---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    user_id,
    COUNT(*) AS val_1,
    AVG(price) AS val_2
FROM smelt.ref('logs')
GROUP BY user_id
HAVING COUNT(*) > 10
