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
    quantity,
    AVG(duration_seconds) AS agg_0,
    AVG(price) AS agg_1,
    MAX(created_at) AS agg_2
FROM smelt.sql_l2_247
GROUP BY quantity
