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
    MAX(created_at) AS metric_1,
    SUM(amount) AS metric_2
FROM smelt.sql_l2_12
GROUP BY DATE_TRUNC('month', event_time)
