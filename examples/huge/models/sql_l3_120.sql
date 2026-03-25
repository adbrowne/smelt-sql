---
materialization: table
incremental:
  enabled: true
  partition_column: event_date
---
SELECT
    DATE_TRUNC('day', event_time) AS period,
    AVG(duration_seconds) AS metric_1,
    AVG(price) AS metric_2
FROM smelt.ref('sql_l2_200')
GROUP BY DATE_TRUNC('day', event_time)
