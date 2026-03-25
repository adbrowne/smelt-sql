---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    COUNT(DISTINCT user_id) AS metric_1,
    AVG(price) AS metric_2
FROM smelt.ref('py_l2_367')
GROUP BY DATE_TRUNC('month', event_time)
