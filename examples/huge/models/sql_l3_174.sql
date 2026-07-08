---
materialization: table
refresh: incremental
grain: partition
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    rating,
    AVG(price) AS agg_0,
    SUM(revenue) AS agg_1,
    MAX(created_at) AS agg_2,
    COUNT(DISTINCT user_id) AS agg_3,
    COUNT(*) AS agg_4
FROM smelt.sql_l2_157
GROUP BY rating
