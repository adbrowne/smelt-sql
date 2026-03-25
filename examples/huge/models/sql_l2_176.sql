---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('day', event_time) AS period,
    SUM(revenue) AS metric_1,
    SUM(quantity) AS metric_2
FROM smelt.ref('py_l1_436')
GROUP BY DATE_TRUNC('day', event_time)
