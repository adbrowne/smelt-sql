---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    DATE_TRUNC('day', event_time) AS period,
    SUM(revenue) AS metric_1,
    AVG(price) AS metric_2
FROM smelt.models.products
GROUP BY DATE_TRUNC('day', event_time)

