---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('day', event_time) AS period,
    SUM(revenue) AS metric_1,
    COUNT(*) AS metric_2
FROM smelt.ref('sql_l2_175')
GROUP BY DATE_TRUNC('day', event_time)
