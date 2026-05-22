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
    session_id,
    AVG(price) AS val_1,
    AVG(amount) AS val_2
FROM smelt.sql_l1_201
GROUP BY session_id
HAVING COUNT(*) > 10
