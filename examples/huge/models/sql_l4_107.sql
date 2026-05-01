---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    amount,
    COUNT(*) AS val_1,
    COUNT(DISTINCT user_id) AS val_2
FROM smelt.models.sql_l3_52
GROUP BY amount
HAVING COUNT(*) > 10

