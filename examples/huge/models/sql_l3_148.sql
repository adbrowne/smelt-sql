---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('week', event_time) AS period,
    AVG(price) AS metric_1,
    MIN(created_at) AS metric_2
FROM smelt.ref('py_l2_252')
GROUP BY DATE_TRUNC('week', event_time)
