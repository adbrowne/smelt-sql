---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    session_id,
    AVG(price) AS val_1,
    SUM(revenue) AS val_2
FROM smelt.sql_l1_100
GROUP BY session_id
HAVING COUNT(*) > 10

