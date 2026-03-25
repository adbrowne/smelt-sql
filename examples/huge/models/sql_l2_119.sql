---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('week', event_time) AS period,
    MAX(created_at) AS metric_1,
    SUM(quantity) AS metric_2
FROM smelt.ref('py_l1_324')
GROUP BY DATE_TRUNC('week', event_time)
