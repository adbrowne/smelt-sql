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
    AVG(amount) AS val_1,
    MIN(created_at) AS val_2
FROM smelt.sql_l3_11
GROUP BY order_id
HAVING COUNT(*) > 10
