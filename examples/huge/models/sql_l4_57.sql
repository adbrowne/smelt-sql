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
    MIN(created_at) AS metric_1,
    AVG(price) AS metric_2
FROM smelt.sql_l3_114
GROUP BY DATE_TRUNC('month', event_time)
