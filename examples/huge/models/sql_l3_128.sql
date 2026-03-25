---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    COUNT(DISTINCT user_id) AS metric_1,
    MAX(created_at) AS metric_2
FROM smelt.ref('py_l2_352')
GROUP BY DATE_TRUNC('month', event_time)
