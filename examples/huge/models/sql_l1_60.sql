---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('day', event_time) AS period,
    SUM(quantity) AS metric_1,
    SUM(amount) AS metric_2
FROM smelt.ref('page_views')
GROUP BY DATE_TRUNC('day', event_time)
