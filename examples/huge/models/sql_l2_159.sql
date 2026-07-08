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
    is_verified,
    SUM(revenue) AS agg_0,
    MIN(created_at) AS agg_1
FROM smelt.sql_l1_14
GROUP BY is_verified
