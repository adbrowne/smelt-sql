---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    page_path,
    AVG(amount) AS val_1,
    COUNT(DISTINCT user_id) AS val_2
FROM smelt.sql_l1_229
GROUP BY page_path
HAVING COUNT(*) > 10
