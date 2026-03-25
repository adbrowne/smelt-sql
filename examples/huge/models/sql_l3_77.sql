---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('week', event_time) AS period,
    MIN(created_at) AS metric_1,
    COUNT(*) AS metric_2
FROM smelt.ref('py_l2_441')
GROUP BY DATE_TRUNC('week', event_time)
