---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    SUM(quantity) AS metric_1,
    SUM(revenue) AS metric_2
FROM smelt.ref('py_l1_326')
GROUP BY DATE_TRUNC('month', event_time)
