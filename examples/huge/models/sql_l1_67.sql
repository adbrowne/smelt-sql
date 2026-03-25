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
    AVG(amount) AS val_1,
    COUNT(DISTINCT user_id) AS val_2
FROM smelt.ref('products')
GROUP BY rating
HAVING COUNT(*) > 10
