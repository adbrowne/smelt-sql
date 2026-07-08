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
    DATE_TRUNC('week', event_time) AS period,
    AVG(price) AS metric_1,
    COUNT(*) AS metric_2
FROM smelt.sql_l1_132
GROUP BY DATE_TRUNC('week', event_time)
