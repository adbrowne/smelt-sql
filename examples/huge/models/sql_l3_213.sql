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
    AVG(amount) AS metric_1,
    SUM(quantity) AS metric_2
FROM smelt.sql_l2_195
GROUP BY DATE_TRUNC('month', event_time)
