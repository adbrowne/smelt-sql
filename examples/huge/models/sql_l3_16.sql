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
    SUM(quantity) AS agg_0,
    AVG(duration_seconds) AS agg_1
FROM smelt.sql_l2_32
GROUP BY quantity
