---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('week', event_time) AS period,
    SUM(revenue) AS metric_1,
    MIN(created_at) AS metric_2
FROM smelt.ref('py_l3_458')
GROUP BY DATE_TRUNC('week', event_time)
