---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('week', event_time) AS period,
    COUNT(*) AS metric_1,
    AVG(amount) AS metric_2
FROM smelt.ref('sql_l3_114')
GROUP BY DATE_TRUNC('week', event_time)
