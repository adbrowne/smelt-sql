---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    MAX(created_at) AS metric_1,
    COUNT(*) AS metric_2
FROM smelt.sql_l3_194
GROUP BY DATE_TRUNC('month', event_time)
