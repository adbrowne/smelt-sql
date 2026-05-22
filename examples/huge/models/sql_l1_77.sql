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
    order_id,
    MIN(created_at) AS agg_0,
    SUM(quantity) AS agg_1,
    SUM(revenue) AS agg_2,
    SUM(amount) AS agg_3,
    COUNT(DISTINCT user_id) AS agg_4
FROM smelt.reviews
GROUP BY order_id
