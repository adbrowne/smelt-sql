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
    DATE_TRUNC('day', event_time) AS period,
    MAX(created_at) AS metric_1,
    MIN(created_at) AS metric_2
FROM smelt.sql_l2_59
GROUP BY DATE_TRUNC('day', event_time)
