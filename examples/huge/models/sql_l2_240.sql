---
materialization: table
incremental:
  enabled: true
  event_time_column: event_time
  partition_column: event_date
  granularity: day
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    SUM(quantity) AS metric_1,
    SUM(revenue) AS metric_2
FROM smelt.ref('sql_l1_101')
GROUP BY DATE_TRUNC('month', event_time)
