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
    revenue,
    AVG(price) AS agg_0,
    AVG(amount) AS agg_1
FROM smelt.sql_l2_16
GROUP BY revenue
