---
materialization: table
refresh: batched
timeseries:
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    DATE_TRUNC('day', event_time) AS period,
    SUM(revenue) AS metric_1,
    COUNT(*) AS metric_2
FROM smelt.sql_l2_238
GROUP BY DATE_TRUNC('day', event_time)
