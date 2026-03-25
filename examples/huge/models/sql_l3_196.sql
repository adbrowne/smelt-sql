---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    MAX(created_at) AS metric_1,
    SUM(amount) AS metric_2
FROM smelt.ref('py_l2_312')
GROUP BY DATE_TRUNC('month', event_time)
