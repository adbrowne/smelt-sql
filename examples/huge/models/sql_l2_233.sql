---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('month', event_time) AS period,
    AVG(duration_seconds) AS metric_1,
    AVG(amount) AS metric_2
FROM smelt.ref('sql_l1_147')
GROUP BY DATE_TRUNC('month', event_time)
