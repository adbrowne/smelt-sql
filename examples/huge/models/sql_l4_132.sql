---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    MIN(created_at) AS metric_1,
    AVG(amount) AS metric_2
FROM smelt.ref('py_l3_349')
GROUP BY DATE_TRUNC('month', event_time)
