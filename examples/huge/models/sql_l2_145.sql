---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('week', event_time) AS period,
    SUM(revenue) AS metric_1,
    AVG(amount) AS metric_2
FROM smelt.ref('sql_l1_178')
GROUP BY DATE_TRUNC('week', event_time)
