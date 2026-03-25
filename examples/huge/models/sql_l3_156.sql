---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    COUNT(*) AS metric_1,
    AVG(duration_seconds) AS metric_2
FROM smelt.ref('sql_l2_173')
GROUP BY DATE_TRUNC('month', event_time)
