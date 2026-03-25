---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('day', event_time) AS period,
    COUNT(*) AS metric_1,
    AVG(price) AS metric_2
FROM smelt.ref('py_l2_388')
GROUP BY DATE_TRUNC('day', event_time)
