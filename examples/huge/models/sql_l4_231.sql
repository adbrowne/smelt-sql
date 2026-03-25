---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('day', event_time) AS period,
    COUNT(*) AS metric_1,
    COUNT(DISTINCT user_id) AS metric_2
FROM smelt.ref('py_l3_326')
GROUP BY DATE_TRUNC('day', event_time)
