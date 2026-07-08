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
    SUM(revenue) AS agg_0,
    SUM(amount) AS agg_1,
    AVG(amount) AS agg_2,
    COUNT(*) AS agg_3,
    SUM(quantity) AS agg_4
FROM smelt.sql_l3_79
GROUP BY event_date
