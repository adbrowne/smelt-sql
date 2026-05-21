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
    discount,
    AVG(price) AS agg_0,
    MAX(created_at) AS agg_1,
    SUM(revenue) AS agg_2,
    MIN(created_at) AS agg_3,
    COUNT(DISTINCT user_id) AS agg_4
FROM smelt.sql_l3_2
GROUP BY discount

