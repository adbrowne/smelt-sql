---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    SUM(amount) AS metric_1,
    SUM(quantity) AS metric_2
FROM smelt.ref('sql_l1_182')
GROUP BY DATE_TRUNC('month', event_time)
