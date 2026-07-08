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
    ip_address,
    AVG(amount) AS agg_0,
    COUNT(DISTINCT user_id) AS agg_1,
    SUM(amount) AS agg_2,
    COUNT(*) AS agg_3,
    MAX(created_at) AS agg_4
FROM smelt.sql_l2_33
GROUP BY ip_address
