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
    event_date,
    COUNT(*) AS agg_0,
    AVG(duration_seconds) AS agg_1,
    SUM(quantity) AS agg_2
FROM smelt.sql_l2_164
GROUP BY event_date
