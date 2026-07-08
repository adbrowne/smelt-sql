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
    SUM(revenue) AS metric_1,
    COUNT(DISTINCT user_id) AS metric_2
FROM smelt.sql_l1_126
GROUP BY DATE_TRUNC('month', event_time)
