---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    order_id,
    SUM(quantity) AS val_1,
    AVG(amount) AS val_2
FROM smelt.sql_l2_65
GROUP BY order_id
HAVING COUNT(*) > 10
