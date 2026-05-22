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
    AVG(price) AS agg_0,
    SUM(quantity) AS agg_1,
    MAX(created_at) AS agg_2,
    COUNT(DISTINCT user_id) AS agg_3,
    AVG(duration_seconds) AS agg_4
FROM smelt.sql_l3_118
GROUP BY order_id
