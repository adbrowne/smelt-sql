---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    SUM(revenue) AS metric_1,
    COUNT(DISTINCT user_id) AS metric_2
FROM smelt.ref('sql_l1_217')
GROUP BY DATE_TRUNC('month', event_time)
