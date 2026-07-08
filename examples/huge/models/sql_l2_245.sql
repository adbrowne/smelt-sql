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
    created_at,
    MAX(created_at) AS agg_0,
    SUM(quantity) AS agg_1,
    SUM(amount) AS agg_2,
    MIN(created_at) AS agg_3
FROM smelt.sql_l1_28
GROUP BY created_at
