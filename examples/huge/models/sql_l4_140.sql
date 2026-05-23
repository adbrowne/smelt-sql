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
    COUNT(*) AS metric_1,
    AVG(amount) AS metric_2
FROM smelt.sql_l3_231
GROUP BY DATE_TRUNC('week', event_time)
