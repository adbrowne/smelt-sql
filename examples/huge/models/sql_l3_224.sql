---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    AVG(duration_seconds) AS metric_1,
    SUM(revenue) AS metric_2
FROM smelt.ref('py_l2_300')
GROUP BY DATE_TRUNC('month', event_time)
