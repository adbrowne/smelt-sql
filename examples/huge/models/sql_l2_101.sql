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
    discount,
    COUNT(DISTINCT user_id) AS agg_0,
    SUM(quantity) AS agg_1,
    MAX(created_at) AS agg_2,
    AVG(duration_seconds) AS agg_3
FROM smelt.sql_l1_205
GROUP BY discount
