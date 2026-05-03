---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    rating,
    COUNT(*) AS val_1,
    AVG(price) AS val_2
FROM smelt.sql_l3_199
GROUP BY rating
HAVING COUNT(*) > 10

