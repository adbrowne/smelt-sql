---
materialization: table
incremental:
  enabled: true
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    MIN(created_at) AS metric_1,
    AVG(amount) AS metric_2
FROM smelt.sql_l3_150
GROUP BY DATE_TRUNC('month', event_time)
