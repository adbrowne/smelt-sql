---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    MIN(created_at) AS metric_1,
    AVG(price) AS metric_2
FROM smelt.ref('py_l3_477')
GROUP BY DATE_TRUNC('month', event_time)
