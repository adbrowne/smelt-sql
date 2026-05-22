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
    SUM(quantity) AS metric_1,
    SUM(amount) AS metric_2
FROM smelt.page_views
GROUP BY DATE_TRUNC('day', event_time)
