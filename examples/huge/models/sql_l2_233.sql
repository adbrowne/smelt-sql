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
    AVG(duration_seconds) AS metric_1,
    AVG(amount) AS metric_2
FROM smelt.sql_l1_51
GROUP BY DATE_TRUNC('month', event_time)
