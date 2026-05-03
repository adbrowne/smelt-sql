---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    product_id,
    COUNT(DISTINCT user_id) AS val_1,
    SUM(quantity) AS val_2
FROM smelt.sql_l2_70
GROUP BY product_id
HAVING COUNT(*) > 10

