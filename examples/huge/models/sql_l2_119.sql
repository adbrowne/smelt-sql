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
    DATE_TRUNC('week', event_time) AS period,
    MAX(created_at) AS metric_1,
    SUM(quantity) AS metric_2
FROM smelt.sql_l1_203
GROUP BY DATE_TRUNC('week', event_time)

