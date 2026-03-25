---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    AVG(amount) AS metric_1,
    SUM(quantity) AS metric_2
FROM smelt.ref('py_l2_387')
GROUP BY DATE_TRUNC('month', event_time)
