---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    price,
    AVG(amount) AS agg_0,
    COUNT(*) AS agg_1,
    COUNT(DISTINCT user_id) AS agg_2,
    SUM(revenue) AS agg_3
FROM smelt.models.sql_l1_92
GROUP BY price

