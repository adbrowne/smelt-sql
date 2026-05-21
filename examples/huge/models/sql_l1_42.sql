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
    COUNT(DISTINCT user_id) AS metric_1,
    SUM(amount) AS metric_2
FROM smelt.orders
GROUP BY DATE_TRUNC('month', event_time)

