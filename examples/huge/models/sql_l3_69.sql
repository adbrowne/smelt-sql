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
    DATE_TRUNC('month', event_time) AS period,
    AVG(price) AS metric_1,
    SUM(revenue) AS metric_2
FROM smelt.sql_l2_245
GROUP BY DATE_TRUNC('month', event_time)
