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
    price,
    AVG(duration_seconds) AS agg_0,
    SUM(amount) AS agg_1,
    COUNT(*) AS agg_2,
    COUNT(DISTINCT user_id) AS agg_3
FROM smelt.sql_l1_160
GROUP BY price
