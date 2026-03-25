---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('day', event_time) AS period,
    SUM(revenue) AS metric_1,
    AVG(price) AS metric_2
FROM smelt.ref('sql_l1_98')
GROUP BY DATE_TRUNC('day', event_time)
