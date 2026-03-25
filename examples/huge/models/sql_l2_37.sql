---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('week', event_time) AS period,
    AVG(price) AS metric_1,
    COUNT(*) AS metric_2
FROM smelt.ref('sql_l1_88')
GROUP BY DATE_TRUNC('week', event_time)
